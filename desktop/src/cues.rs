use std::fs::File;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bukospeak_core::cues::Cue;
use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::proto::control::SOUNDS;
use cpal::DeviceId;
use rodio::buffer::SamplesBuffer;
use rodio::source::Zero;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::audio;

pub fn folder() -> Option<PathBuf> {
    dirs::data_local_dir().map(|folder| folder.join("bukospeak").join(SOUNDS))
}

const MONO: NonZero<u16> = NonZero::new(1).unwrap();
const RATE: NonZero<u32> = NonZero::new(SAMPLE_RATE).unwrap();

fn tone(cue: Cue) -> SamplesBuffer {
    SamplesBuffer::new(MONO, RATE, cue.tone())
}

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
            return self.player.append(tone(cue));
        };
        match File::open(&file).map_err(CueError::from).and_then(|opened| Ok(Decoder::try_from(opened)?)) {
            Ok(sound) => self.player.append(sound),
            Err(error) => {
                tracing::warn!("{}: {error}", file.display());
                self.player.append(tone(cue));
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
