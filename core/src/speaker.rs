//! The render callback's end of a call: the network task's frames in, the mix out.

use std::sync::Arc;
use std::time::Instant;

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::FRAME_SAMPLES;
use rtrb::Consumer;

use crate::echo::Tap;
use crate::jitter::Packet;
use crate::playout::Mixer;
use crate::speaking::Speaking;

/// What the network task hands the speaker.
#[expect(clippy::large_enum_variant, reason = "the ring holds frames by value, so delivering one never allocates")]
pub(crate) enum Downlink {
    Frame(MemberId, Packet),
    Left(MemberId),
}

/// What the platform's render callback plays: 48 kHz mono, in buffers of any length. Nothing
/// here allocates, locks or waits.
pub struct Speaker {
    mixer: Mixer,
    downlink: Consumer<Downlink>,
    frame: [f32; FRAME_SAMPLES],
    played: usize,
    speaking: Arc<Speaking>,
    /// Where the mix goes for the microphone, if its echo is the engine's to cancel.
    tap: Option<Tap>,
    /// Whether the platform's audio stopped calling [`pull`](Self::pull) since it last did.
    stopped: bool,
}

impl Speaker {
    pub(crate) fn new(mixer: Mixer, downlink: Consumer<Downlink>, speaking: Arc<Speaking>, tap: Option<Tap>) -> Self {
        Self { mixer, downlink, frame: [0.0; FRAME_SAMPLES], played: FRAME_SAMPLES, speaking, tap, stopped: false }
    }

    /// Fills samples for the platform to play, the first of them at `plays`, as the platform
    /// tells it.
    pub fn pull(&mut self, mut samples: &mut [f32], plays: Instant) {
        if self.stopped {
            self.stopped = false;
            self.flush();
        }
        if let Some(tap) = &mut self.tap {
            tap.plays(samples.len(), plays);
        }
        while !samples.is_empty() {
            if self.played == FRAME_SAMPLES {
                self.render();
            }
            let take = (FRAME_SAMPLES - self.played).min(samples.len());
            let (now, rest) = samples.split_at_mut(take);
            now.copy_from_slice(&self.frame[self.played..][..take]);
            self.played += take;
            samples = rest;
        }
    }

    /// For when the platform's audio stops calling [`pull`](Self::pull): nobody shows as
    /// speaking meanwhile, and what arrived by the time it resumes is too late to play.
    pub fn stopped(&mut self) {
        if let Some(tap) = &mut self.tap {
            tap.stopped();
        }
        self.stopped = true;
        self.speaking.set_voices(std::iter::empty());
    }

    fn flush(&mut self) {
        while let Ok(delivered) = self.downlink.pop() {
            if let Downlink::Left(member) = delivered {
                self.mixer.remove(member);
            }
        }
        self.mixer.flush();
        self.played = FRAME_SAMPLES;
    }

    /// Mixes the next frame.
    fn render(&mut self) {
        while let Ok(delivered) = self.downlink.pop() {
            match delivered {
                // Every other member the server admits has a voice, so the insert can't fail.
                Downlink::Frame(from, packet) => _ = self.mixer.insert(from, packet),
                Downlink::Left(member) => self.mixer.remove(member),
            }
        }
        self.mixer.render(&mut self.frame);
        if let Some(tap) = &mut self.tap {
            tap.push(&self.frame);
        }
        self.played = 0;
        self.speaking.set_voices(self.mixer.speaking());
    }
}

#[cfg(test)]
mod tests {
    use rtrb::RingBuffer;

    use super::*;
    use crate::signal::talkspurt;

    #[test]
    fn a_stopped_speaker_shows_nobody_and_resumes_without_what_arrived_meanwhile() {
        let (mut downlink, delivered) = RingBuffer::new(100);
        let speaking = Arc::new(Speaking::default());
        let mut speaker = Speaker::new(Mixer::new().unwrap(), delivered, Arc::clone(&speaking), None);
        let (member, you) = (MemberId(2), MemberId(1));
        let packets = talkspurt(60, 440.0, 0.5);
        let mut played = [0.0; FRAME_SAMPLES];
        for packet in &packets[..20] {
            downlink.push(Downlink::Frame(member, *packet)).unwrap();
            speaker.pull(&mut played, Instant::now());
        }
        assert_eq!(speaking.members(you), [member]);
        assert!(played.iter().any(|&sample| sample != 0.0));

        speaker.stopped();
        assert!(speaking.members(you).is_empty());
        for packet in &packets[20..] {
            downlink.push(Downlink::Frame(member, *packet)).unwrap();
        }
        speaker.pull(&mut played, Instant::now());
        assert!(played.iter().all(|&sample| sample == 0.0));
        assert!(speaking.members(you).is_empty());
    }
}
