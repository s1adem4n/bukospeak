use std::sync::Arc;
use std::time::Duration;

use bukospeak_core::Channel;
use bukospeak_core::cues::{self, Cue, Seen};
use bukospeak_core::proto::audio::SAMPLE_RATE;
use rtrb::{Consumer, Producer, RingBuffer};

const MOST_AT_ONCE: usize = 3;

const BATCHES: usize = 8;

#[derive(Clone)]
pub struct Sounds([Arc<[f32]>; Cue::ALL.len()]);

impl Default for Sounds {
    fn default() -> Self {
        Self(Cue::ALL.map(|cue| cue.tone().into()))
    }
}

impl Sounds {
    fn of(&self, cue: Cue) -> &[f32] {
        &self.0[cue as usize]
    }

    fn length(&self, cue: Cue) -> Duration {
        Duration::from_secs_f64(self.of(cue).len() as f64 / f64::from(SAMPLE_RATE))
    }
}

#[derive(Clone, Copy, Default)]
struct Batch {
    cues: [Option<Cue>; MOST_AT_ONCE],
    ends_call: bool,
}

impl Batch {
    fn of(cues: &[Cue], ends_call: bool) -> Self {
        let mut batch = Self { ends_call, ..Self::default() };
        for (slot, &cue) in batch.cues.iter_mut().zip(cues) {
            *slot = Some(cue);
        }
        batch
    }
}

#[derive(Default)]
pub struct Cues {
    seen: Seen,
    muted: bool,
    open: bool,
    sounds: Sounds,
    batches: Option<Producer<Batch>>,
}

impl Cues {
    pub fn player(&mut self) -> Player {
        let (batches, played) = RingBuffer::new(BATCHES);
        self.batches = Some(batches);
        self.open = true;
        Player { sounds: self.sounds.clone(), batches: played, playing: Batch::default(), cue: 0, at: 0 }
    }

    pub fn set_sound(&mut self, cue: Cue, samples: Option<Vec<f32>>) {
        self.sounds.0[cue as usize] = samples.unwrap_or_else(|| cue.tone()).into();
    }

    pub fn observe(&mut self, channel: &Channel) {
        // A call already left can still tell its last updates.
        if self.open {
            self.see(Seen::of(channel, self.muted), false);
        }
    }

    pub fn mute(&mut self, muted: bool) {
        self.muted = muted;
        self.see(Seen { muted, ..self.seen.clone() }, false);
    }

    pub fn leave(&mut self) -> Duration {
        self.open = false;
        let farewell = if self.seen.in_call { self.sounds.length(Cue::Left) } else { Duration::ZERO };
        self.see(Seen::default(), true);
        farewell
    }

    fn see(&mut self, seen: Seen, ends_call: bool) {
        let changed = cues::changes(&self.seen, &seen);
        self.seen = seen;
        if changed.is_empty() {
            return;
        }
        if let Some(batches) = &mut self.batches {
            // A full ring means the audio isn't playing, so the batches behind are stale anyway.
            let _ = batches.push(Batch::of(&changed, ends_call));
        }
    }
}

pub struct Player {
    sounds: Sounds,
    batches: Consumer<Batch>,
    playing: Batch,
    cue: usize,
    at: usize,
}

impl Player {
    pub fn ends_call(&self) -> bool {
        self.playing.ends_call
    }

    pub fn mix(&mut self, mut samples: &mut [f32]) {
        // Only the latest change is worth hearing: queued behind earlier ones, a cue would come late.
        while let Ok(batch) = self.batches.pop() {
            (self.playing, self.cue, self.at) = (batch, 0, 0);
        }
        while let Some(&Some(cue)) = self.playing.cues.get(self.cue) {
            let sound = &self.sounds.of(cue)[self.at..];
            let take = sound.len().min(samples.len());
            for (sample, cue) in samples.iter_mut().zip(&sound[..take]) {
                *sample += cue;
            }
            samples = &mut samples[take..];
            if take == sound.len() {
                (self.cue, self.at) = (self.cue + 1, 0);
            } else {
                self.at += take;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bukospeak_core::State;
    use bukospeak_core::proto::MemberId;
    use bukospeak_core::proto::control::Member;

    use super::*;

    fn member(id: u16) -> Member {
        Member { id: MemberId(id), name: format!("member {id}"), muted: false, reconnecting: false }
    }

    fn connected(members: &[u16]) -> Channel {
        Channel::shown(State::Connected, Some(MemberId(1)), members.iter().copied().map(member).collect(), Vec::new())
    }

    fn played(player: &mut Player, frames: usize) -> Vec<f32> {
        let mut samples = vec![0.; frames];
        player.mix(&mut samples);
        samples
    }

    #[test]
    fn the_sounds_are_the_cues_in_order() {
        let sounds = Sounds::default();
        for cue in Cue::ALL {
            assert_eq!(sounds.of(cue), cue.tone(), "{cue:?}");
        }
    }

    #[test]
    fn joining_plays_its_sound_into_the_call() {
        let mut cues = Cues::default();
        let mut player = cues.player();
        cues.observe(&connected(&[1]));
        let joined = Cue::Joined.tone();
        let mut heard = played(&mut player, 100);
        heard.extend(played(&mut player, joined.len()));
        assert_eq!(heard[..joined.len()], joined);
        assert!(heard[joined.len()..].iter().all(|&sample| sample == 0.));
    }

    #[test]
    fn a_change_replaces_a_cue_still_playing() {
        let mut cues = Cues::default();
        let mut player = cues.player();
        cues.observe(&connected(&[1]));
        played(&mut player, 100);
        cues.mute(true);
        let muted = Cue::Muted.tone();
        assert_eq!(played(&mut player, muted.len()), muted);
    }

    #[test]
    fn leaving_plays_the_farewell_over_a_silent_call() {
        let mut cues = Cues::default();
        let mut player = cues.player();
        cues.observe(&connected(&[1, 2]));
        played(&mut player, Cue::Joined.tone().len());
        assert!(!player.ends_call());
        let farewell = cues.leave();
        assert_eq!(farewell, Sounds::default().length(Cue::Left));
        let left = Cue::Left.tone();
        assert_eq!(played(&mut player, left.len()), left);
        assert!(player.ends_call());
        cues.observe(&connected(&[1, 2]));
        assert!(played(&mut player, 100).iter().all(|&sample| sample == 0.));
    }

    #[test]
    fn leaving_outside_a_call_has_no_farewell() {
        assert_eq!(Cues::default().leave(), Duration::ZERO);
    }
}
