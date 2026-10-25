//! The capture callback's end of a call: microphone samples in, frames for the uplink out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::Thread;

use bukospeak_proto::audio::FRAME_SAMPLES;
use rtrb::Producer;

use crate::capture::{Capture, Encoded};
use crate::speaking::Speaking;

/// The engine's settings the capture follows.
pub(crate) struct Settings {
    pub noise_suppression: AtomicBool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { noise_suppression: AtomicBool::new(true) }
    }
}

/// What the platform's capture callback feeds: 48 kHz mono, in buffers of any length. Nothing
/// here allocates, locks or waits; it wakes the uplink thread when there is a frame to send.
pub struct Microphone {
    capture: Capture,
    frame: [f32; FRAME_SAMPLES],
    filled: usize,
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
        frames: Producer<Encoded>,
        uplink: Thread,
        settings: Arc<Settings>,
        speaking: Arc<Speaking>,
    ) -> Self {
        Self { capture, frame: [0.0; FRAME_SAMPLES], filled: 0, settings, speaking, frames, uplink: Uplink(uplink) }
    }

    pub fn push(&mut self, mut samples: &[f32]) {
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

    fn process(&mut self) {
        self.capture.set_noise_suppression(self.settings.noise_suppression.load(Ordering::Relaxed));
        let mut last_sent = None;
        let processed = self.capture.process(&self.frame, |frame| {
            // A full ring means the uplink fell a second behind: the frame is lost, as it would
            // be on the way.
            let _ = self.frames.push(*frame);
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

#[cfg(test)]
mod tests {
    use std::thread;

    use bukospeak_proto::MemberId;
    use bukospeak_proto::audio::SAMPLE_RATE;
    use rtrb::RingBuffer;

    use super::*;
    use crate::signal::voice;

    #[test]
    fn speech_in_buffers_of_any_length_is_framed_sent_and_shown_as_speaking() {
        let (frames, mut sent) = RingBuffer::new(1000);
        let speaking = Arc::new(Speaking::default());
        let mut microphone = Microphone::new(Capture::new().unwrap(), frames, thread::current(), Arc::default(), Arc::clone(&speaking));

        let signal = voice(SAMPLE_RATE as usize, 0.3);
        let mut spoke = false;
        // 441 samples, as a 44.1 kHz device's 10 ms would be, never line up with a frame.
        for buffer in signal.chunks(441) {
            microphone.push(buffer);
            spoke |= speaking.members(MemberId(1)) == [MemberId(1)];
        }
        assert!(spoke);

        let indices: Vec<u32> = std::iter::from_fn(|| sent.pop().ok()).map(|frame| frame.index).collect();
        assert!(indices.len() > 50, "only {} frames sent", indices.len());
        assert!(indices.windows(2).all(|pair| pair[1] == pair[0] + 1), "frames missing or out of order");
    }
}
