use std::time::Instant;

use bukospeak_core::Speaker;

use crate::cues::Player;

pub struct Output {
    speaker: Speaker,
    cues: Player,
}

impl Output {
    pub fn new(speaker: Speaker, cues: Player) -> Self {
        Self { speaker, cues }
    }

    pub fn pull(&mut self, samples: &mut [f32], plays: Instant) {
        if self.cues.ends_call() {
            samples.fill(0.);
        } else {
            self.speaker.pull(samples, plays);
        }
        self.cues.mix(samples);
    }

    pub fn stopped(&mut self) {
        self.speaker.stopped();
    }
}
