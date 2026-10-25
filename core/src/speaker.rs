//! The render callback's end of a call: the network task's frames in, the mix out.

use std::sync::Arc;

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::FRAME_SAMPLES;
use rtrb::Consumer;

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
}

impl Speaker {
    pub(crate) fn new(mixer: Mixer, downlink: Consumer<Downlink>, speaking: Arc<Speaking>) -> Self {
        Self { mixer, downlink, frame: [0.0; FRAME_SAMPLES], played: FRAME_SAMPLES, speaking }
    }

    pub fn pull(&mut self, mut samples: &mut [f32]) {
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

    /// Forgets the frames held and arriving so far, for a speaker that resumes after its audio
    /// stopped: they are too late to play.
    pub fn flush(&mut self) {
        while let Ok(delivered) = self.downlink.pop() {
            if let Downlink::Left(member) = delivered {
                self.mixer.remove(member);
            }
        }
        self.mixer.flush();
        self.played = FRAME_SAMPLES;
        self.speaking.set_voices(std::iter::empty());
    }

    fn render(&mut self) {
        while let Ok(delivered) = self.downlink.pop() {
            match delivered {
                // Every other member the server admits has a voice, so the insert can't fail.
                Downlink::Frame(from, packet) => _ = self.mixer.insert(from, packet),
                Downlink::Left(member) => self.mixer.remove(member),
            }
        }
        self.mixer.render(&mut self.frame);
        self.played = 0;
        self.speaking.set_voices(self.mixer.speaking());
    }
}
