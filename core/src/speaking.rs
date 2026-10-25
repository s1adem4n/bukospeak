//! Who speaks right now, written by the audio callbacks and read by the engine's watcher.

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use bukospeak_proto::MemberId;

use crate::playout::VOICES;

/// Lock-free, so the realtime callbacks can write it every frame.
#[derive(Default)]
pub struct Speaking {
    /// The members whose voice plays, in voice order; 0 marks a free slot, since the server
    /// never hands out id 0.
    voices: [AtomicU16; VOICES],
    you: AtomicBool,
}

impl Speaking {
    /// For the speaker, after each frame it rendered.
    pub fn set_voices(&self, members: impl Iterator<Item = MemberId>) {
        let mut members = members.map(|member| member.0);
        for voice in &self.voices {
            voice.store(members.next().unwrap_or(0), Ordering::Relaxed);
        }
    }

    /// For the microphone, after each frame it captured.
    pub fn set_you(&self, speaking: bool) {
        self.you.store(speaking, Ordering::Relaxed);
    }

    /// Everyone speaking, `you` among them if this member does, in id order.
    pub fn members(&self, you: MemberId) -> Vec<MemberId> {
        let mut members: Vec<MemberId> =
            self.voices.iter().map(|voice| voice.load(Ordering::Relaxed)).filter(|&id| id != 0).map(MemberId).collect();
        if self.you.load(Ordering::Relaxed) {
            members.push(you);
        }
        members.sort_unstable();
        members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_voices_playing_and_whether_you_speak() {
        let speaking = Speaking::default();
        speaking.set_voices([MemberId(5), MemberId(2)].into_iter());
        assert_eq!(speaking.members(MemberId(3)), [MemberId(2), MemberId(5)]);

        speaking.set_you(true);
        speaking.set_voices([MemberId(5)].into_iter());
        assert_eq!(speaking.members(MemberId(3)), [MemberId(3), MemberId(5)]);
    }
}
