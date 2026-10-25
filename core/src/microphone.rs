//! The capture callback's end of a call: microphone samples in, frames for the uplink out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::Thread;
use std::time::Instant;

use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES};
use rtrb::Producer;

use crate::capture::{Capture, Encoded};
use crate::echo::Reference;
use crate::speaking::Speaking;

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

/// What the platform's capture callback feeds: 48 kHz mono, in buffers of any length. Nothing
/// here allocates, locks or waits; it wakes the uplink thread when there is a frame to send.
pub struct Microphone {
    capture: Capture,
    /// What the speaker played, where its echo is the engine's to cancel, and the frame of it
    /// read for the frame captured.
    played: Option<Reference>,
    played_frame: [f32; FRAME_SAMPLES],
    frame: [f32; FRAME_SAMPLES],
    filled: usize,
    /// When the platform's audio last stopped calling [`push`](Self::push), until it resumes.
    stopped_at: Option<Instant>,
    settings: Arc<Settings>,
    speaking: Arc<Speaking>,
    // Fields drop in order: the ring is abandoned before the uplink is woken a last time, so it
    // sees that and ends.
    frames: Producer<Encoded>,
    uplink: Uplink,
}

struct Uplink(Thread);

impl Drop for Uplink {
    fn drop(&mut self) {
        self.0.unpark();
    }
}

impl Microphone {
    pub(crate) fn new(
        capture: Capture,
        played: Option<Reference>,
        frames: Producer<Encoded>,
        uplink: Thread,
        settings: Arc<Settings>,
        speaking: Arc<Speaking>,
    ) -> Self {
        Self {
            capture,
            played,
            played_frame: [0.0; FRAME_SAMPLES],
            frame: [0.0; FRAME_SAMPLES],
            filled: 0,
            stopped_at: None,
            settings,
            speaking,
            frames,
            uplink: Uplink(uplink),
        }
    }

    pub fn push(&mut self, mut samples: &[f32]) {
        if let Some(played) = &mut self.played {
            played.captured(Instant::now(), samples.len());
        }
        if let Some(stopped_at) = self.stopped_at.take() {
            // Truncated, the count wraps as the index does after 497 days anyway.
            self.capture.skip((stopped_at.elapsed().as_micros() / FRAME_DURATION.as_micros()) as u32);
        }
        while !samples.is_empty() {
            let take = (FRAME_SAMPLES - self.filled).min(samples.len());
            let (now, rest) = samples.split_at(take);
            self.frame[self.filled..][..take].copy_from_slice(now);
            self.filled += take;
            samples = rest;
            if self.filled == FRAME_SAMPLES {
                self.filled = 0;
                self.process();
            }
        }
    }

    /// Ends what the capture left unfinished, for when the platform's audio stops calling
    /// [`push`](Self::push): a talkspurt gets its last frame, and this member shows as quiet.
    /// Once it resumes, the frames meanwhile count as not captured.
    pub fn stopped(&mut self) {
        if let Some(played) = &mut self.played {
            played.stopped();
        }
        self.filled = 0;
        self.stopped_at = Some(Instant::now());
        let mut sent = false;
        // As in a frame pushed, libopus fails only on arguments the fixed frame size rules out.
        let _ = self.capture.end(|frame| {
            send(&mut self.frames, frame);
            sent = true;
        });
        self.speaking.set_you(false);
        if sent {
            self.uplink.0.unpark();
        }
    }

    fn process(&mut self) {
        self.capture.set_noise_suppression(self.settings.noise_suppression.load(Ordering::Relaxed));
        self.capture.set_echo_cancellation(self.settings.echo_cancellation.load(Ordering::Relaxed));
        if self.settings.muted.load(Ordering::Relaxed) {
            // Muted, the microphone hears silence, and the gate ends a talkspurt as in a pause.
            self.frame.fill(0.0);
        }
        let played = match &mut self.played {
            Some(played) => {
                played.read(&mut self.played_frame);
                Some(&self.played_frame)
            }
            None => None,
        };
        let mut last_sent = None;
        let processed = self.capture.process(&self.frame, played, |frame| {
            send(&mut self.frames, frame);
            last_sent = Some(frame.last);
        });
        // libopus fails only on arguments the fixed frame size rules out; such a frame is lost.
        if processed.is_err() {
            return;
        }
        self.speaking.set_you(last_sent == Some(false));
        if last_sent.is_some() {
            self.uplink.0.unpark();
        }
    }
}

fn send(frames: &mut Producer<Encoded>, frame: &Encoded) {
    // A full ring means the uplink fell a second behind: the frame is lost, as it would be on
    // the way.
    let _ = frames.push(*frame);
}

#[cfg(test)]
mod tests {
    use std::thread;

    use bukospeak_proto::MemberId;
    use bukospeak_proto::audio::{Level, SAMPLE_RATE};
    use rtrb::{Consumer, RingBuffer};

    use super::*;
    use crate::conditioner::Conditioning;
    use crate::signal::voice;

    fn microphone(settings: Arc<Settings>) -> (Microphone, Consumer<Encoded>, Arc<Speaking>) {
        let (frames, sent) = RingBuffer::new(1000);
        let speaking = Arc::new(Speaking::default());
        (
            Microphone::new(
                Capture::new(Conditioning::Platform).unwrap(),
                None,
                frames,
                thread::current(),
                settings,
                Arc::clone(&speaking),
            ),
            sent,
            speaking,
        )
    }

    fn speaks(speaking: &Speaking) -> bool {
        speaking.members(MemberId(1)) == [MemberId(1)]
    }

    #[test]
    fn speech_in_buffers_of_any_length_is_framed_sent_and_shown_as_speaking() {
        let (mut microphone, mut sent, speaking) = microphone(Arc::default());

        let signal = voice(SAMPLE_RATE as usize, 0.3);
        let mut spoke = false;
        // 441 samples, as a 44.1 kHz device's 10 ms would be, never line up with a frame.
        for buffer in signal.chunks(441) {
            microphone.push(buffer);
            spoke |= speaks(&speaking);
        }
        assert!(spoke);

        let indices: Vec<u32> = std::iter::from_fn(|| sent.pop().ok()).map(|frame| frame.index).collect();
        assert!(indices.len() > 50, "only {} frames sent", indices.len());
        assert!(indices.windows(2).all(|pair| pair[1] == pair[0] + 1), "frames missing or out of order");
    }

    #[test]
    fn stopping_in_speech_ends_the_talkspurt_and_shows_quiet() {
        let (mut microphone, mut sent, speaking) = microphone(Arc::default());
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        microphone.push(&[0.1; 100]);
        assert!(speaks(&speaking));
        microphone.stopped();
        assert!(!speaks(&speaking));
        let frames: Vec<Encoded> = std::iter::from_fn(|| sent.pop().ok()).collect();
        assert_eq!(frames.iter().filter(|frame| frame.last).count(), 1);
        assert!(frames.last().unwrap().last);
    }

    #[test]
    fn muted_speech_ends_after_the_hold_like_a_pause() {
        let settings = Arc::new(Settings::default());
        let (mut microphone, mut sent, speaking) = microphone(Arc::clone(&settings));
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        assert!(speaks(&speaking));
        settings.muted.store(true, Ordering::Relaxed);
        let before = std::iter::from_fn(|| sent.pop().ok()).count();
        assert!(before > 50);

        microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        assert!(!speaks(&speaking));
        let after: Vec<Encoded> = std::iter::from_fn(|| sent.pop().ok()).collect();
        // The hold's 300 ms, silent once the denoiser's tail faded out, the last marking the end.
        assert_eq!(after.len(), 30);
        assert!(after[10..].iter().all(|frame| frame.level == Level::SILENT));
        assert!(after.last().unwrap().last);
    }

    #[test]
    fn resuming_counts_the_time_stopped_into_the_index() {
        let (mut microphone, mut sent, _) = microphone(Arc::default());
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        microphone.stopped();
        let ended = std::iter::from_fn(|| sent.pop().ok()).last().unwrap().index;
        thread::sleep(std::time::Duration::from_millis(500));
        microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        let resumed = sent.pop().unwrap().index;
        // 50 frames went by, then the input settles anew before the gate opens with its pre-roll.
        assert!(resumed >= ended + 50, "resumed at frame {resumed} after {ended}");
    }
}
