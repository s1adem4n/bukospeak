use std::f32::consts::TAU;
use std::fs::File;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bukospeak_core::Error as CallError;
use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::proto::control::{Rejection, SOUNDS};
use cpal::DeviceId;
use rodio::buffer::SamplesBuffer;
use rodio::source::Zero;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::audio;
use crate::install::{self, Error};

pub fn folder() -> Option<PathBuf> {
    dirs::data_local_dir().map(|folder| folder.join("bukospeak").join(SOUNDS))
}

pub fn sync(server: &str, folder: &Path) -> Result<(), Error> {
    match install::install(server, SOUNDS, folder, |_| ()) {
        Err(Error::Download(CallError::Rejected(Rejection::Unavailable))) => Ok(install::remove(folder)?),
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

    fn tone(self) -> SamplesBuffer {
        SamplesBuffer::new(MONO, RATE, self.render(SAMPLE_RATE))
    }

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

const MONO: NonZero<u16> = NonZero::new(1).unwrap();
const RATE: NonZero<u32> = NonZero::new(SAMPLE_RATE).unwrap();

// Lets the device's buffer play out what the mixer handed it before the output closes.
const TAIL: Duration = Duration::from_millis(250);

// The output stays open between cues: a device that idles swallows the start of a stream.
pub struct Cues {
    asked: mpsc::Sender<Ask>,
}

enum Ask {
    Open(Option<DeviceId>),
    Play(Vec<Cue>, Option<DeviceId>),
    Close,
}

impl Cues {
    pub fn new(folder: Option<PathBuf>) -> std::io::Result<Self> {
        let (asked, asks) = mpsc::channel();
        thread::Builder::new().name("bukospeak-cues".into()).spawn(move || {
            let mut output = None;
            for ask in asks {
                match ask {
                    Ask::Open(device) => {
                        Output::on(&mut output, device);
                    }
                    Ask::Play(cues, device) => {
                        if let Some(output) = Output::on(&mut output, device) {
                            output.replace(&cues, folder.as_deref());
                        }
                    }
                    Ask::Close => {
                        if let Some(output) = output.take() {
                            output.close();
                        }
                    }
                }
            }
        })?;
        Ok(Self { asked })
    }

    pub fn open(&self, output: Option<DeviceId>) {
        self.ask(Ask::Open(output));
    }

    pub fn play(&self, cues: Vec<Cue>, output: Option<DeviceId>) {
        self.ask(Ask::Play(cues, output));
    }

    pub fn close(&self) {
        self.ask(Ask::Close);
    }

    fn ask(&self, ask: Ask) {
        // The thread runs as long as this.
        let _ = self.asked.send(ask);
    }
}

struct Output {
    device: Option<DeviceId>,
    _sink: MixerDeviceSink,
    player: Player,
    broke: Arc<AtomicBool>,
}

impl Output {
    fn on(output: &mut Option<Self>, device: Option<DeviceId>) -> Option<&mut Self> {
        if output.as_ref().is_none_or(|open| open.device != device || open.broke.load(Ordering::Relaxed)) {
            *output = Self::open(device).inspect_err(|error| tracing::warn!("no sound: {error}")).ok();
        }
        output.as_mut()
    }

    fn open(device: Option<DeviceId>) -> Result<Self, CueError> {
        let broke = Arc::new(AtomicBool::new(false));
        let mut sink = DeviceSinkBuilder::from_device(audio::output_device(device.as_ref())?)?
            .with_error_callback({
                let broke = broke.clone();
                move |error| {
                    tracing::warn!("the sounds' output: {error}");
                    broke.store(true, Ordering::Relaxed);
                }
            })
            .open_sink_or_fallback()?;
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        Ok(Self { device, _sink: sink, player, broke })
    }

    // Only the latest change is worth hearing: queued behind earlier ones, a cue would come late.
    fn replace(&self, cues: &[Cue], folder: Option<&Path>) {
        self.player.clear();
        cues.iter().for_each(|&cue| self.append(cue, folder));
        self.player.play();
    }

    fn append(&self, cue: Cue, folder: Option<&Path>) {
        let file = folder.map(|folder| folder.join(cue.file()).with_extension("wav")).filter(|file| file.exists());
        let Some(file) = file else {
            return self.player.append(cue.tone());
        };
        match File::open(&file).map_err(CueError::from).and_then(|opened| Ok(Decoder::try_from(opened)?)) {
            Ok(sound) => self.player.append(sound),
            Err(error) => {
                tracing::warn!("{}: {error}", file.display());
                self.player.append(cue.tone());
            }
        }
    }

    fn close(self) {
        self.player.append(Zero::new(MONO, RATE).take_duration(TAIL));
        self.player.sleep_until_end();
    }
}

#[derive(Debug, thiserror::Error)]
enum CueError {
    #[error(transparent)]
    Device(#[from] audio::Error),
    #[error(transparent)]
    Sink(#[from] rodio::DeviceSinkError),
    #[error(transparent)]
    File(#[from] std::io::Error),
    #[error(transparent)]
    Wav(#[from] rodio::decoder::DecoderError),
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
