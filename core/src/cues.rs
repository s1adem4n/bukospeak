use std::f32::consts::TAU;
use std::path::Path;

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::SAMPLE_RATE;
use bukospeak_proto::control::{Rejection, SOUNDS};

use crate::channel::{Channel, State};
use crate::install::{self, Error};
use crate::session;

pub fn sync(server: &str, folder: &Path) -> Result<(), Error> {
    match install::install(server, SOUNDS, folder, |_| ()) {
        Err(Error::Download(session::Error::Rejected(Rejection::Unavailable))) => Ok(install::remove(folder)?),
        installed => installed,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Joined,
    Left,
    Came,
    Went,
    Muted,
    Unmuted,
}

type Note = (f32, f32);

const E5: f32 = 659.26;
const A4: f32 = 440.;
const B5: f32 = 987.77;
const E4: f32 = 329.63;

const NOTE: f32 = 0.32;
const ATTACK: f32 = 0.006;
const FADE: f32 = 0.07;
const STEP: f32 = 0.085;

impl Cue {
    pub const ALL: [Self; 6] = [Self::Joined, Self::Left, Self::Came, Self::Went, Self::Muted, Self::Unmuted];

    pub fn file(self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Left => "left",
            Self::Came => "came",
            Self::Went => "went",
            Self::Muted => "muted",
            Self::Unmuted => "unmuted",
        }
    }

    pub fn tone(self) -> Vec<f32> {
        let notes = self.notes();
        let rate = SAMPLE_RATE as f32;
        let end = notes.iter().map(|&(_, start)| start + NOTE).fold(0., f32::max);
        let mut tone: Vec<f32> = (0..(end * rate).ceil() as usize)
            .map(|index| {
                let time = index as f32 / rate;
                notes
                    .iter()
                    .filter(|&&(_, start)| time >= start)
                    .map(|&(pitch, start)| {
                        let since = time - start;
                        let envelope = (since / ATTACK).min(1.) * (-(since - ATTACK).max(0.) / FADE).exp();
                        // A touch of the octave makes it a soft bell rather than a beep.
                        envelope * ((TAU * pitch * since).sin() + 0.2 * (2. * TAU * pitch * since).sin())
                    })
                    .sum()
            })
            .collect();
        let loudest = tone.iter().fold(0_f32, |most, sample| most.max(sample.abs()));
        let gain = self.level() / loudest;
        tone.iter_mut().for_each(|sample| *sample *= gain);
        tone
    }

    fn notes(self) -> &'static [Note] {
        match self {
            Self::Joined => &[(E5, 0.), (B5, STEP)],
            Self::Left => &[(B5, 0.), (E5, STEP)],
            Self::Came => &[(A4, 0.), (E5, STEP)],
            Self::Went => &[(E5, 0.), (A4, STEP)],
            Self::Muted => &[(E4, 0.)],
            Self::Unmuted => &[(A4, 0.)],
        }
    }

    fn level(self) -> f32 {
        match self {
            Self::Joined | Self::Left => 0.14,
            Self::Came | Self::Went | Self::Muted | Self::Unmuted => 0.1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub in_call: bool,
    pub connected: bool,
    pub others: Vec<MemberId>,
    pub muted: bool,
}

impl Seen {
    pub fn of(channel: &Channel, muted: bool) -> Self {
        let you = channel.you();
        Self {
            in_call: channel.state() != State::Out,
            connected: channel.state() == State::Connected,
            others: channel.members().iter().map(|member| member.id).filter(|id| Some(*id) != you).collect(),
            muted,
        }
    }
}

pub fn changes(before: &Seen, after: &Seen) -> Vec<Cue> {
    let mut cues = Vec::new();
    match (before.in_call, after.in_call) {
        (false, true) => cues.push(Cue::Joined),
        (true, false) => cues.push(Cue::Left),
        // Who is in the call is known only while connected; joining again, it is empty.
        _ if before.connected && after.connected => {
            if after.others.iter().any(|id| !before.others.contains(id)) {
                cues.push(Cue::Came);
            }
            if before.others.iter().any(|id| !after.others.contains(id)) {
                cues.push(Cue::Went);
            }
        }
        _ => {}
    }
    if before.in_call && after.in_call {
        match (before.muted, after.muted) {
            (false, true) => cues.push(Cue::Muted),
            (true, false) => cues.push(Cue::Unmuted),
            _ => {}
        }
    }
    cues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(in_call: bool, others: &[u16], muted: bool) -> Seen {
        Seen { in_call, connected: in_call, others: others.iter().copied().map(MemberId).collect(), muted }
    }

    #[test]
    fn a_cue_is_quiet_and_fades_out() {
        for cue in Cue::ALL {
            let sound = cue.tone();
            let loudest = sound.iter().fold(0_f32, |most, sample| most.max(sample.abs()));
            assert!((loudest - cue.level()).abs() < 1e-6, "{cue:?} peaks at {loudest}");
            let tail = &sound[sound.len() - 480..];
            assert!(tail.iter().all(|sample| sample.abs() < 0.01), "{cue:?} ends in silence");
        }
    }

    #[test]
    fn what_begins_rises_and_what_ends_falls() {
        let pitches = |cue: Cue| cue.notes().iter().map(|&(pitch, _)| pitch).collect::<Vec<_>>();
        for (begins, ends) in [(Cue::Joined, Cue::Left), (Cue::Came, Cue::Went)] {
            assert!(pitches(begins).is_sorted());
            assert_eq!(pitches(ends), pitches(begins).into_iter().rev().collect::<Vec<_>>());
        }
    }

    #[test]
    fn joining_leaving_others_and_muting_each_have_their_cue() {
        let with_one = seen(true, &[2], false);
        assert_eq!(changes(&seen(false, &[], false), &with_one), [Cue::Joined]);
        assert_eq!(changes(&with_one, &seen(false, &[], false)), [Cue::Left]);
        assert_eq!(changes(&seen(true, &[], false), &with_one), [Cue::Came]);
        assert_eq!(changes(&with_one, &seen(true, &[], false)), [Cue::Went]);
        assert_eq!(changes(&with_one, &seen(true, &[2], true)), [Cue::Muted]);
        assert_eq!(changes(&seen(true, &[2], true), &with_one), [Cue::Unmuted]);
    }

    #[test]
    fn muting_is_heard_only_in_a_call() {
        assert_eq!(changes(&seen(false, &[], false), &seen(false, &[], true)), []);
        assert_eq!(changes(&seen(true, &[2], true), &seen(false, &[], false)), [Cue::Left]);
    }

    #[test]
    fn joining_again_tells_no_one_coming_or_going() {
        let connected = seen(true, &[2], false);
        let reconnecting = Seen { connected: false, others: Vec::new(), ..connected.clone() };
        assert_eq!(changes(&connected, &reconnecting), []);
        assert_eq!(changes(&reconnecting, &connected), []);
    }
}
