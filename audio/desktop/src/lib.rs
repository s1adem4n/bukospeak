//! A call on the desktop's audio devices, through cpal.
//!
//! [`Audio`] plays a call's microphone and speaker on an input and an output device, the
//! system's defaults or chosen ones. A thread of its own holds the streams and builds them anew
//! when a device is lost or the stream breaks, retrying every second until the device is back.

mod convert;
mod lease;

use std::fmt;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::{Microphone, Speaker};
pub use cpal::DeviceId;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, ErrorKind, Host, InputCallbackInfo, OutputCallbackInfo, SampleFormat, Stream, StreamConfig, SupportedBufferSize,
    SupportedStreamConfigRange,
};

use crate::convert::Format;
use crate::lease::{Half, Lease};

/// How long a stream that couldn't be built waits before the next try.
const RETRY: Duration = Duration::from_secs(1);

/// The rate the device runs at if it can't run at the call's: CD audio's, which many run at.
const CD_RATE: u32 = 44_100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Input => "input",
            Self::Output => "output",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the {0} device {1} isn't there")]
    Missing(Direction, DeviceId),
    #[error("there is no {0} device")]
    NoDevice(Direction),
    #[error("the {0} device offers no 32-bit float format")]
    NoFormat(Direction),
    #[error("the {0} device: {1}")]
    Format(Direction, convert::Error),
    #[error("the {0} device: {1}")]
    Device(Direction, cpal::Error),
}

/// An audio device, as the settings list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    /// Whether it is the system's default, which [`Audio`] uses unless told otherwise.
    pub is_default: bool,
}

/// The devices there are for a direction.
pub fn devices(direction: Direction) -> Result<Vec<Device>, Error> {
    let host = cpal::default_host();
    let fail = |error| Error::Device(direction, error);
    let (default, devices): (_, Vec<cpal::Device>) = match direction {
        Direction::Input => (host.default_input_device(), host.input_devices().map_err(fail)?.collect()),
        Direction::Output => (host.default_output_device(), host.output_devices().map_err(fail)?.collect()),
    };
    let default = default.and_then(|device| device.id().ok());
    // A device without an id was unplugged while it was listed.
    Ok(devices
        .into_iter()
        .filter_map(|device| {
            let id = device.id().ok()?;
            Some(Device { is_default: default.as_ref() == Some(&id), name: device.to_string(), id })
        })
        .collect())
}

/// A call playing on the devices, until it is dropped.
pub struct Audio {
    commands: mpsc::Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

enum Command {
    /// Use this device, or the default for None.
    Use(Direction, Option<DeviceId>),
    /// The stream built as this generation broke.
    Broke(Direction, u64, cpal::Error),
    Stop,
}

impl Audio {
    /// Starts playing the call on the devices, the defaults for None. `report` hears why a
    /// device doesn't play, once each time it stops.
    pub fn start(
        microphone: Microphone,
        speaker: Speaker,
        input: Option<DeviceId>,
        output: Option<DeviceId>,
        report: impl Fn(Error) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let input = Side::new(Direction::Input, input, microphone);
        let output = Side::new(Direction::Output, output, speaker);
        let sender = commands.clone();
        let thread =
            thread::Builder::new().name("bukospeak-audio".into()).spawn(move || run(input, output, &received, &sender, &report))?;
        Ok(Self { commands, thread: Some(thread) })
    }

    pub fn set_input(&self, device: Option<DeviceId>) {
        self.send(Command::Use(Direction::Input, device));
    }

    pub fn set_output(&self, device: Option<DeviceId>) {
        self.send(Command::Use(Direction::Output, device));
    }

    fn send(&self, command: Command) {
        // The audio thread ends only when told to stop, which drop does.
        self.commands.send(command).expect("the audio thread runs until the audio is dropped");
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the audio thread doesn't panic");
        }
    }
}

fn run(
    mut input: Side<Microphone>,
    mut output: Side<Speaker>,
    received: &mpsc::Receiver<Command>,
    commands: &mpsc::Sender<Command>,
    report: &dyn Fn(Error),
) {
    loop {
        input.start(commands, report, build_input);
        output.start(commands, report, build_output);
        let command = if input.stream.is_some() && output.stream.is_some() {
            received.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            received.recv_timeout(RETRY)
        };
        match command {
            Ok(Command::Use(Direction::Input, device)) => input.switch(device),
            Ok(Command::Use(Direction::Output, device)) => output.switch(device),
            Ok(Command::Broke(Direction::Input, generation, error)) => input.broke(generation, error, report),
            Ok(Command::Broke(Direction::Output, generation, error)) => output.broke(generation, error, report),
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// One direction's device and stream, and the call's half the stream plays.
struct Side<T> {
    direction: Direction,
    device: Option<DeviceId>,
    stream: Option<Stream>,
    /// Counts the streams built, so a break reported late by a stream replaced since is ignored.
    generation: u64,
    /// The half comes back here whenever the stream that played it is gone.
    back: mpsc::Sender<T>,
    idle: mpsc::Receiver<T>,
    /// Whether the user heard why the device doesn't play.
    reported: bool,
}

/// Builds a stream for a half on the chosen or default device; `broke` is its error callback.
type Build<T> = fn(&Host, Option<&DeviceId>, Lease<T>, Box<dyn FnMut(cpal::Error) + Send>) -> Result<Stream, Error>;

impl<T: Half> Side<T> {
    fn new(direction: Direction, device: Option<DeviceId>, half: T) -> Self {
        let (back, idle) = mpsc::channel();
        back.send(half).expect("the channel was just made");
        Self { direction, device, stream: None, generation: 0, back, idle, reported: false }
    }

    fn start(&mut self, commands: &mpsc::Sender<Command>, report: &dyn Fn(Error), build: Build<T>) {
        if self.stream.is_some() {
            return;
        }
        let half = self.idle.try_recv().expect("a half no stream plays is back");
        self.generation += 1;
        let (direction, generation, commands) = (self.direction, self.generation, commands.clone());
        let broke = Box::new(move |error: cpal::Error| match error.kind() {
            // A glitch, a stream the backend rerouted itself, or one that runs without
            // realtime priority: all play on.
            ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {}
            // The audio thread is gone only once the audio stopped, with this stream.
            _ => _ = commands.send(Command::Broke(direction, generation, error)),
        });
        match build(&cpal::default_host(), self.device.as_ref(), Lease::new(half, self.back.clone()), broke) {
            Ok(stream) => {
                self.stream = Some(stream);
                self.reported = false;
            }
            Err(error) => self.report(error, report),
        }
    }

    fn switch(&mut self, device: Option<DeviceId>) {
        self.device = device;
        self.stream = None;
        self.reported = false;
    }

    fn broke(&mut self, generation: u64, error: cpal::Error, report: &dyn Fn(Error)) {
        if generation == self.generation && self.stream.take().is_some() {
            self.report(Error::Device(self.direction, error), report);
        }
    }

    fn report(&mut self, error: Error, report: &dyn Fn(Error)) {
        if !self.reported {
            self.reported = true;
            report(error);
        }
    }
}

fn build_input(
    host: &Host,
    id: Option<&DeviceId>,
    mut microphone: Lease<Microphone>,
    broke: Box<dyn FnMut(cpal::Error) + Send>,
) -> Result<Stream, Error> {
    let direction = Direction::Input;
    let device = device(direction, id, host.default_input_device(), host)?;
    let config = config(direction, device.supported_input_configs().map_err(|error| Error::Device(direction, error))?)?;
    let mut input = convert::Input::new(format(config)).map_err(|error| Error::Format(direction, error))?;
    let stream = device
        .build_input_stream(
            config,
            move |samples: &[f32], info: &InputCallbackInfo| {
                // The stream's clock may have an origin of its own; how long ago the samples
                // were captured is the same on any.
                let timestamp = info.timestamp();
                let captured = Instant::now() - timestamp.callback.duration_since(timestamp.capture);
                input.push(samples, captured, |frame, captured| microphone.push(frame, captured));
            },
            broke,
            None,
        )
        .map_err(|error| Error::Device(direction, error))?;
    stream.play().map_err(|error| Error::Device(direction, error))?;
    Ok(stream)
}

fn build_output(
    host: &Host,
    id: Option<&DeviceId>,
    mut speaker: Lease<Speaker>,
    broke: Box<dyn FnMut(cpal::Error) + Send>,
) -> Result<Stream, Error> {
    let direction = Direction::Output;
    let device = device(direction, id, host.default_output_device(), host)?;
    let config = config(direction, device.supported_output_configs().map_err(|error| Error::Device(direction, error))?)?;
    let mut output = convert::Output::new(format(config)).map_err(|error| Error::Format(direction, error))?;
    let stream = device
        .build_output_stream(
            config,
            move |samples: &mut [f32], info: &OutputCallbackInfo| {
                let timestamp = info.timestamp();
                let plays = Instant::now() + timestamp.playback.duration_since(timestamp.callback);
                output.pull(samples, plays, |frame, plays| speaker.pull(frame, plays));
            },
            broke,
            None,
        )
        .map_err(|error| Error::Device(direction, error))?;
    stream.play().map_err(|error| Error::Device(direction, error))?;
    Ok(stream)
}

fn device(direction: Direction, id: Option<&DeviceId>, default: Option<cpal::Device>, host: &Host) -> Result<cpal::Device, Error> {
    match id {
        Some(id) => host.device_by_id(id).ok_or_else(|| Error::Missing(direction, id.clone())),
        None => default.ok_or(Error::NoDevice(direction)),
    }
}

/// The device's float format closest to the call's: 48 kHz if it has it, fewest channels, and
/// 10 ms buffers if it takes them.
fn config(direction: Direction, ranges: impl Iterator<Item = SupportedStreamConfigRange>) -> Result<StreamConfig, Error> {
    let range = ranges
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .min_by_key(|range| (!range.contains_rate(SAMPLE_RATE), !range.contains_rate(CD_RATE), range.channels()))
        .ok_or(Error::NoFormat(direction))?;
    let rate = [SAMPLE_RATE, CD_RATE].into_iter().find(|&rate| range.contains_rate(rate)).unwrap_or(range.max_sample_rate());
    let frames = rate / 100;
    let buffer_size = match *range.buffer_size() {
        SupportedBufferSize::Range { min, max } if (min..=max).contains(&frames) => BufferSize::Fixed(frames),
        _ => BufferSize::Default,
    };
    Ok(StreamConfig { channels: range.channels(), sample_rate: rate, buffer_size })
}

fn format(config: StreamConfig) -> Format {
    Format { channels: usize::from(config.channels), rate: config.sample_rate }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(format: SampleFormat, channels: u16, rates: (u32, u32), buffer_size: SupportedBufferSize) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, rates.0, rates.1, buffer_size, format)
    }

    fn chosen(ranges: &[SupportedStreamConfigRange]) -> Result<StreamConfig, Error> {
        config(Direction::Input, ranges.iter().copied())
    }

    #[test]
    fn a_float_format_is_chosen_at_the_call_rate_else_cd_rate_then_with_fewest_channels() {
        let any = SupportedBufferSize::Unknown;
        let stereo_48 = range(SampleFormat::F32, 2, (8_000, 48_000), any);
        let mono_cd = range(SampleFormat::F32, 1, (44_100, 44_100), any);
        let mono_96 = range(SampleFormat::F32, 1, (96_000, 96_000), any);
        let mono_48_i16 = range(SampleFormat::I16, 1, (48_000, 48_000), any);

        let config = chosen(&[mono_cd, mono_48_i16, stereo_48]).unwrap();
        assert_eq!((config.channels, config.sample_rate), (2, SAMPLE_RATE));
        let config = chosen(&[mono_96, mono_cd]).unwrap();
        assert_eq!((config.channels, config.sample_rate), (1, CD_RATE));
        assert_eq!(chosen(&[mono_96]).unwrap().sample_rate, 96_000);
        assert!(matches!(chosen(&[mono_48_i16]), Err(Error::NoFormat(Direction::Input))));
    }

    #[test]
    fn buffers_are_10_ms_where_the_device_takes_them() {
        let with = |buffer_size| chosen(&[range(SampleFormat::F32, 1, (48_000, 48_000), buffer_size)]).unwrap().buffer_size;
        assert_eq!(with(SupportedBufferSize::Range { min: 64, max: 4096 }), BufferSize::Fixed(480));
        assert_eq!(with(SupportedBufferSize::Range { min: 1024, max: 4096 }), BufferSize::Default);
        assert_eq!(with(SupportedBufferSize::Unknown), BufferSize::Default);
    }
}
