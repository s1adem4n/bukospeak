//! The app's sounds, for while its window isn't looked at: when one joins or leaves the call,
//! the other comes or goes, and the microphone mutes or unmutes. Each is a soft tone, unless the
//! folder of sounds has a WAV file of its name, which plays instead.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use bukospeak_core::proto::audio::SAMPLE_RATE;
use cpal::DeviceId;

use crate::audio;

/// What a sound tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Joined,
    Left,
    /// Another member came into the call.
    Came,
    /// Another member went.
    Went,
    Muted,
    Unmuted,
}

/// A tone of a cue: its pitch in Hz, and when it starts, in seconds.
type Note = (f32, f32);

const E5: f32 = 659.26;
const A4: f32 = 440.;
const B5: f32 = 987.77;
const E4: f32 = 329.63;

/// How long a note sounds, its attack, and how fast it fades after, in seconds.
const NOTE: f32 = 0.32;
const ATTACK: f32 = 0.006;
const FADE: f32 = 0.07;
/// The time between a cue's two notes.
const STEP: f32 = 0.085;

impl Cue {
    /// The name of its file in the folder of sounds, without `.wav`.
    fn file(self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Left => "left",
            Self::Came => "came",
            Self::Went => "went",
            Self::Muted => "muted",
            Self::Unmuted => "unmuted",
        }
    }

    /// Its notes: up for what begins, down for what ends; one's own a fifth higher than the
    /// other's.
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

    /// How loud it is at its loudest, of full scale: quiet, beside the call's voices.
    fn level(self) -> f32 {
        match self {
            Self::Joined | Self::Left => 0.14,
            Self::Came | Self::Went | Self::Muted | Self::Unmuted => 0.1,
        }
    }

    /// Its tone, in the call's format.
    fn tone(self) -> Arc<[f32]> {
        self.render(SAMPLE_RATE).into()
    }

    /// Its tone at `rate`, in one channel, at its level at its loudest.
    fn render(self, rate: u32) -> Vec<f32> {
        let notes = self.notes();
        let end = notes.iter().map(|&(_, start)| start + NOTE).fold(0., f32::max);
        let length = (end * rate as f32).ceil() as usize;
        let mut tone: Vec<f32> = (0..length)
            .map(|index| {
                let time = index as f32 / rate as f32;
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
}

/// Plays cues one after another, on a thread of its own, as long as this lives.
pub struct Cues {
    asked: mpsc::Sender<(Cue, Option<DeviceId>)>,
}

impl Cues {
    /// Cues from the WAV files in `folder` where there are, tones where not; the files are read
    /// each time, so ones put there meanwhile play from then on.
    pub fn new(folder: PathBuf) -> std::io::Result<Self> {
        let (asked, cues) = mpsc::channel::<(Cue, Option<DeviceId>)>();
        thread::Builder::new().name("bukospeak-cues".into()).spawn(move || {
            for (cue, output) in cues {
                let file = folder.join(cue.file()).with_extension("wav");
                let sound = match file.exists().then(|| audio::read_sound(&file)) {
                    Some(Ok(sound)) => sound,
                    Some(Err(error)) => {
                        tracing::warn!("{}: {error}", file.display());
                        cue.tone()
                    }
                    None => cue.tone(),
                };
                match audio::sound(output.as_ref(), sound) {
                    // The stream plays while it lives; the moment after lets its last buffer out.
                    Ok((_stream, length)) => thread::sleep(length + Duration::from_millis(100)),
                    Err(error) => tracing::warn!("no sound: {error}"),
                }
            }
        })?;
        Ok(Self { asked })
    }

    /// Plays `cue` on the output device, the default for None.
    pub fn play(&self, cue: Cue, output: Option<DeviceId>) {
        // The thread ends only with the app, when no cue is to be heard any more.
        let _ = self.asked.send((cue, output));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cue_is_quiet_and_fades_out() {
        for cue in [Cue::Joined, Cue::Left, Cue::Came, Cue::Went, Cue::Muted, Cue::Unmuted] {
            let sound = cue.render(48_000);
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
}
