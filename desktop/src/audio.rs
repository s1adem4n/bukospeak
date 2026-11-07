mod convert;
mod lease;

use std::fmt;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::{Meter, Microphone, Speaker};
pub use cpal::DeviceId;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, ErrorKind, Host, InputCallbackInfo, OutputCallbackInfo, SampleFormat, Stream, StreamConfig, SupportedBufferSize,
    SupportedStreamConfigRange,
};

use crate::audio::convert::Format;
use crate::audio::lease::{Half, Lease};

const RETRY: Duration = Duration::from_secs(1);

// PulseAudio drops a stream's callback on a thread of its own, after deleting the stream.
const HAND_BACK: Duration = Duration::from_millis(250);

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub is_default: bool,
}

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

pub struct Audio {
    commands: mpsc::Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

enum Command {
    Use(Direction, Option<DeviceId>),
    Broke(Direction, u64, cpal::Error),
    Stop,
}

impl Audio {
    pub fn call(
        microphone: Microphone,
        speaker: Speaker,
        meter: Option<Meter>,
        input: Option<DeviceId>,
        output: Option<DeviceId>,
        report: impl Fn(Error) + Send + 'static,
    ) -> std::io::Result<Self> {
        let capture = Capture { meter, microphone: Some(microphone) };
        Self::start(Side::new(Direction::Input, input, capture), Some(Side::new(Direction::Output, output, speaker)), report)
    }

    pub fn listen(meter: Meter, input: Option<DeviceId>, report: impl Fn(Error) + Send + 'static) -> std::io::Result<Self> {
        let capture = Capture { meter: Some(meter), microphone: None };
        Self::start(Side::new(Direction::Input, input, capture), None, report)
    }

    fn start(input: Side<Capture>, output: Option<Side<Speaker>>, report: impl Fn(Error) + Send + 'static) -> std::io::Result<Self> {
        let (commands, received) = mpsc::channel();
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
    mut input: Side<Capture>,
    mut output: Option<Side<Speaker>>,
    received: &mpsc::Receiver<Command>,
    commands: &mpsc::Sender<Command>,
    report: &dyn Fn(Error),
) {
    loop {
        input.start(commands, report, build_input);
        if let Some(output) = &mut output {
            output.start(commands, report, build_output);
        }
        let command = if input.stream.is_some() && output.as_ref().is_none_or(|output| output.stream.is_some()) {
            received.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            received.recv_timeout(RETRY)
        };
        match command {
            Ok(Command::Use(Direction::Input, device)) => input.switch(device),
            Ok(Command::Use(Direction::Output, device)) => {
                if let Some(output) = &mut output {
                    output.switch(device);
                }
            }
            Ok(Command::Broke(Direction::Input, generation, error)) => input.broke(generation, error, report),
            Ok(Command::Broke(Direction::Output, generation, error)) => {
                if let Some(output) = &mut output {
                    output.broke(generation, error, report);
                }
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

struct Capture {
    meter: Option<Meter>,
    microphone: Option<Microphone>,
}

impl Capture {
    fn push(&mut self, samples: &[f32], captured: Instant) {
        if let Some(meter) = &self.meter {
            meter.measure(samples);
        }
        if let Some(microphone) = &mut self.microphone {
            microphone.push(samples, captured);
        }
    }
}

impl Half for Capture {
    fn stopped(&mut self) {
        if let Some(meter) = &self.meter {
            meter.reset();
        }
        if let Some(microphone) = &mut self.microphone {
            microphone.stopped();
        }
    }
}

impl Half for Speaker {
    fn stopped(&mut self) {
        Speaker::stopped(self);
    }
}

struct Side<T> {
    direction: Direction,
    device: Option<DeviceId>,
    stream: Option<Stream>,
    generation: u64,
    back: mpsc::Sender<T>,
    handed_back: mpsc::Receiver<T>,
    reported: bool,
}

type Build<T> = fn(&Host, Option<&DeviceId>, Lease<T>, Box<dyn FnMut(cpal::Error) + Send>) -> Result<Stream, Error>;

impl<T: Half> Side<T> {
    fn new(direction: Direction, device: Option<DeviceId>, half: T) -> Self {
        let (back, handed_back) = mpsc::channel();
        back.send(half).expect("the channel was just made");
        Self { direction, device, stream: None, generation: 0, back, handed_back, reported: false }
    }

    fn start(&mut self, commands: &mpsc::Sender<Command>, report: &dyn Fn(Error), build: Build<T>) {
        if self.stream.is_some() {
            return;
        }
        let Ok(half) = self.handed_back.recv_timeout(HAND_BACK) else {
            return;
        };
        self.generation += 1;
        let (direction, generation, commands) = (self.direction, self.generation, commands.clone());
        let broke = Box::new(move |error: cpal::Error| match error.kind() {
            // A glitch, a stream the backend rerouted, or one without realtime priority play on.
            ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {}
            // The audio thread outlives its streams.
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
    mut capture: Lease<Capture>,
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
                // The stream's clock may have an origin of its own, so only the difference counts.
                let timestamp = info.timestamp();
                let captured = Instant::now() - timestamp.callback.duration_since(timestamp.capture);
                input.push(samples, captured, |frame, captured| capture.push(frame, captured));
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

pub fn output_device(id: Option<&DeviceId>) -> Result<cpal::Device, Error> {
    let host = cpal::default_host();
    device(Direction::Output, id, host.default_output_device(), &host)
}

fn device(direction: Direction, id: Option<&DeviceId>, default: Option<cpal::Device>, host: &Host) -> Result<cpal::Device, Error> {
    match id {
        Some(id) => host.device_by_id(id).ok_or_else(|| Error::Missing(direction, id.clone())),
        None => default.ok_or(Error::NoDevice(direction)),
    }
}

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
