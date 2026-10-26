//! The capture callback's end of a call: microphone samples in, frames for the uplink out.
//!
//! It frames the samples, takes out the rumble and the speaker's echo where the engine
//! conditions the signal, numbers the frames on the sender's timeline, and hands them to the
//! uplink thread, which processes the rest, as a callback might not in time. Nothing here
//! allocates, locks or waits; it wakes the uplink thread when there is a frame to process.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::Thread;
use std::time::Instant;

use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES};
use rtrb::Producer;

use crate::capture::Captured;
use crate::conditioner::{Conditioning, Filter};
use crate::echo::Tap;

/// The engine's settings the capture follows.
pub(crate) struct Settings {
    pub noise_suppression: AtomicBool,
    pub echo_cancellation: AtomicBool,
    pub muted: AtomicBool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { noise_suppression: AtomicBool::new(true), echo_cancellation: AtomicBool::new(true), muted: AtomicBool::new(false) }
    }
}

/// What the platform's capture callback feeds: 48 kHz mono, in buffers of any length.
pub struct Microphone {
    /// None where the platform's audio conditions the signal.
    filter: Option<Filter>,
    frame: [f32; FRAME_SAMPLES],
    filled: usize,
    next_index: u32,
    /// When the platform's audio last stopped calling [`push`](Self::push), until it resumes.
    stopped_at: Option<Instant>,
    settings: Arc<Settings>,
    // Fields drop in order: the ring is abandoned before the uplink is woken a last time, so it
    // sees that and ends.
    frames: Producer<Captured>,
    uplink: Uplink,
}

struct Uplink(Thread);

impl Drop for Uplink {
    fn drop(&mut self) {
        self.0.unpark();
    }
}

impl Microphone {
    /// Makes the microphone, and where the engine conditions the signal, the speaker's end of
    /// what it played, whose echo the microphone cancels.
    pub(crate) fn new(
        conditioning: Conditioning,
        frames: Producer<Captured>,
        uplink: Thread,
        settings: Arc<Settings>,
    ) -> (Self, Option<Tap>) {
        let (filter, tap) = match conditioning {
            Conditioning::Engine => {
                let (filter, tap) = Filter::new();
                (Some(filter), Some(tap))
            }
            Conditioning::Platform => (None, None),
        };
        let microphone = Self {
            filter,
            frame: [0.0; FRAME_SAMPLES],
            filled: 0,
            next_index: 0,
            stopped_at: None,
            settings,
            frames,
            uplink: Uplink(uplink),
        };
        (microphone, tap)
    }

    /// Takes samples the platform captured, the first of them at `captured`, as the platform
    /// tells it.
    pub fn push(&mut self, mut samples: &[f32], captured: Instant) {
        if let Some(filter) = &mut self.filter {
            filter.captured(samples.len(), captured);
        }
        if let Some(stopped_at) = self.stopped_at.take() {
            // The frames meanwhile count as not captured, so the index stays the sender's clock:
            // listeners measure delay against it. Truncated, the count wraps as the index does
            // after 497 days anyway.
            self.next_index = self.next_index.wrapping_add((stopped_at.elapsed().as_micros() / FRAME_DURATION.as_micros()) as u32);
        }
        let queued = Instant::now();
        let mut handed = false;
        while !samples.is_empty() {
            let take = (FRAME_SAMPLES - self.filled).min(samples.len());
            let (now, rest) = samples.split_at(take);
            self.frame[self.filled..][..take].copy_from_slice(now);
            self.filled += take;
            samples = rest;
            if self.filled == FRAME_SAMPLES {
                self.filled = 0;
                self.hand_over(queued);
                handed = true;
            }
        }
        if handed {
            self.uplink.0.unpark();
        }
    }

    /// Tells the uplink that the platform's audio stopped calling [`push`](Self::push), so it
    /// ends the speech the input cut short. Once it resumes, the frames meanwhile count as not
    /// captured.
    pub fn stopped(&mut self) {
        self.filled = 0;
        self.stopped_at = Some(Instant::now());
        if let Some(filter) = &mut self.filter {
            filter.stopped();
        }
        let index = self.take_index();
        // Frames leave the ring's last slot free, so a full ring ends in a stop already, with
        // only lost frames after it: this one would tell the uplink nothing new.
        let _ = self.frames.push(Captured::Stopped { index });
        self.uplink.0.unpark();
    }

    fn hand_over(&mut self, queued: Instant) {
        let index = self.take_index();
        let mut samples = if self.settings.muted.load(Ordering::Relaxed) {
            // Muted, the microphone hears silence, and the gate ends speech as in a pause.
            [0.0; FRAME_SAMPLES]
        } else {
            // A NaN, infinite or huge sample from a faulty driver would turn RNNoise's state to NaN
            // for good, and NaN in its spectrum is undefined behavior in its FFT: clip to full
            // scale, as an integer device would, and take NaN for silence.
            self.frame.map(|sample| if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) })
        };
        // Filtered even if it is lost, so the echo canceller's timeline stays the input's.
        if let Some(filter) = &mut self.filter {
            filter.set_echo_cancellation(self.settings.echo_cancellation.load(Ordering::Relaxed));
            filter.filter(&mut samples);
        }
        // The ring's last slot is kept for a stop, so the uplink always learns where the input
        // stopped. Without room beyond it the uplink fell far behind: the frame is lost, as it
        // would be on the way, and its missing index tells the uplink so.
        if self.frames.slots() > 1 {
            // With a slot free, the push can't fail.
            let _ = self.frames.push(Captured::Frame { index, queued, samples });
        }
    }

    /// The next frame's index. It counts a frame lost on the way to the uplink too, so the
    /// timeline stays true.
    fn take_index(&mut self) -> u32 {
        let index = self.next_index;
        self.next_index = self.next_index.wrapping_add(1);
        index
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use bukospeak_proto::audio::SAMPLE_RATE;
    use rtrb::{Consumer, RingBuffer};

    use super::*;
    use crate::signal::voice;

    fn microphone(slots: usize, conditioning: Conditioning, settings: Arc<Settings>) -> (Microphone, Consumer<Captured>) {
        let (frames, captured) = RingBuffer::new(slots);
        (Microphone::new(conditioning, frames, thread::current(), settings).0, captured)
    }

    fn drain(captured: &mut Consumer<Captured>) -> Vec<Captured> {
        std::iter::from_fn(|| captured.pop().ok()).collect()
    }

    fn index(captured: &Captured) -> u32 {
        match *captured {
            Captured::Frame { index, .. } | Captured::Stopped { index } => index,
        }
    }

    #[test]
    fn buffers_of_any_length_are_framed_and_numbered_in_order() {
        let (mut microphone, mut captured) = microphone(1000, Conditioning::Platform, Arc::default());
        let signal = voice(SAMPLE_RATE as usize, 0.3);
        // 441 samples, as a 44.1 kHz device's 10 ms would be, never line up with a frame.
        for buffer in signal.chunks(441) {
            microphone.push(buffer, Instant::now());
        }
        let frames = drain(&mut captured);
        assert_eq!(frames.iter().map(index).collect::<Vec<_>>(), (0..100).collect::<Vec<_>>());
        for (captured, expected) in frames.iter().zip(signal.as_chunks::<FRAME_SAMPLES>().0) {
            let Captured::Frame { samples, .. } = captured else { panic!("a stop amid frames") };
            assert_eq!(samples, expected);
        }
    }

    #[test]
    fn broken_samples_are_clipped_or_silenced() {
        let (mut microphone, mut captured) = microphone(10, Conditioning::Platform, Arc::default());
        let mut frame = [0.1; FRAME_SAMPLES];
        frame[..4].copy_from_slice(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30]);
        microphone.push(&frame, Instant::now());
        let [Captured::Frame { samples, .. }] = drain(&mut captured)[..] else { panic!("not one frame") };
        assert_eq!(samples[..5], [0.0, 1.0, -1.0, -1.0, 0.1]);
    }

    #[test]
    fn muted_the_frames_are_silent() {
        let settings = Arc::new(Settings::default());
        let (mut microphone, mut captured) = microphone(10, Conditioning::Engine, Arc::clone(&settings));
        settings.muted.store(true, Ordering::Relaxed);
        microphone.push(&voice(FRAME_SAMPLES, 0.3), Instant::now());
        let [Captured::Frame { samples, .. }] = drain(&mut captured)[..] else { panic!("not one frame") };
        assert!(samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_stop_keeps_an_index_free_and_resuming_counts_the_time_stopped() {
        let (mut microphone, mut captured) = microphone(1000, Conditioning::Platform, Arc::default());
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3), Instant::now());
        microphone.push(&[0.1; 100], Instant::now());
        microphone.stopped();
        let before = drain(&mut captured);
        assert!(matches!(before.last(), Some(Captured::Stopped { index: 100 })), "{:?}", before.last().map(index));

        thread::sleep(Duration::from_millis(500));
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3), Instant::now());
        // The samples before the stop that didn't fill a frame are dropped.
        let resumed = index(&drain(&mut captured)[0]);
        assert!(resumed >= 101 + 50, "resumed at frame {resumed}");
    }

    #[test]
    fn a_full_ring_keeps_its_last_slot_for_a_stop() {
        let (mut microphone, mut captured) = microphone(3, Conditioning::Platform, Arc::default());
        microphone.push(&[0.1; 3 * FRAME_SAMPLES], Instant::now());
        microphone.stopped();
        assert_eq!(drain(&mut captured).iter().map(index).collect::<Vec<_>>(), [0, 1, 3], "the third frame is lost");

        // A stop in the last slot already ends what came before; the frames after it are lost.
        microphone.push(&[0.1; 3 * FRAME_SAMPLES], Instant::now());
        microphone.stopped();
        microphone.push(&[0.1; FRAME_SAMPLES], Instant::now());
        microphone.stopped();
        let after = drain(&mut captured);
        assert!(matches!(after[..], [Captured::Frame { .. }, Captured::Frame { .. }, Captured::Stopped { .. }]), "{after:?}");
    }

    #[test]
    fn conditioned_frames_are_handed_over_without_allocating() {
        let (mut microphone, mut captured) = microphone(1000, Conditioning::Engine, Arc::default());
        let signal = voice(2 * SAMPLE_RATE as usize, 0.3);
        let start = Instant::now();
        assert_no_alloc::assert_no_alloc(|| {
            for (n, buffer) in signal.chunks(441).enumerate() {
                microphone.push(buffer, start + Duration::from_micros(9_187 * n as u64));
            }
            microphone.stopped();
        });
        assert_eq!(drain(&mut captured).len(), 201);
    }
}
