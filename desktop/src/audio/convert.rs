//! Between a device's format and the call's 48 kHz mono: channels mixed down or copied out, and
//! the rate converted in 10 ms chunks when the device runs at another, and the times the
//! platform tells for the device's samples carried over to the call's. Nothing here allocates
//! after construction.
//!
//! The input and the output device each run on their own clock; the few ppm between them and
//! the senders' clocks are the jitter buffers' to absorb, so the ratio here is fixed.

use std::time::{Duration, Instant};

use bukospeak_core::proto::audio::{FRAME_SAMPLES, SAMPLE_RATE};
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Resampler};

/// A device's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub channels: usize,
    pub rate: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the device runs at {0} Hz, which doesn't divide into 10 ms chunks")]
    Rate(u32),
    #[error("the device has no channels")]
    NoChannels,
    #[error(transparent)]
    Resampler(#[from] rubato::ResamplerConstructionError),
}

/// How much of a channel's level the last 10 ms make: it follows over about a second.
const LEVEL_SMOOTHING: f32 = 0.01;

/// From the input device's samples to the call's.
pub struct Input {
    channels: usize,
    rate: u32,
    /// 10 ms at the device's rate, mixed down to mono, and when its first sample was captured.
    chunk: Box<[f32]>,
    captured: Instant,
    filled: usize,
    mixdown: Mixdown,
    resampling: Option<Resampling>,
}

/// Mixes channels down weighted by their levels, so a microphone on one input of an interface
/// keeps its level beside the silent others, which an average would halve and add their noise
/// to, while channels alike are averaged.
struct Mixdown {
    weights: Box<[f32]>,
    /// The channels' mean squares, following over about a second.
    levels: Box<[f32]>,
    /// The channels' sums of squares in the chunk filled.
    energies: Box<[f32]>,
}

/// From the call's samples to the output device's.
pub struct Output {
    channels: usize,
    rate: u32,
    /// 10 ms at the device's rate, in mono.
    chunk: Box<[f32]>,
    played: usize,
    resampling: Option<Resampling>,
}

/// A fixed-ratio resampler between 10 ms at the device's rate and a frame at 48 kHz.
struct Resampling {
    fft: Fft<f32>,
    frame: Box<[f32]>,
    /// How much later a sample comes out than it went in.
    delay: Duration,
}

impl Input {
    pub fn new(device: Format) -> Result<Self, Error> {
        let chunk = chunk_len(device)?;
        let resampling = Resampling::new(device.rate, SAMPLE_RATE)?;
        let (channels, rate) = (device.channels, device.rate);
        // Told anew with the first sample of each chunk.
        let captured = Instant::now();
        let mixdown = Mixdown::new(channels);
        Ok(Self { channels, rate, chunk: vec![0.0; chunk].into(), captured, filled: 0, mixdown, resampling })
    }

    /// Takes the device's interleaved samples, the first of them captured at `captured`, and
    /// hands `call` them at 48 kHz mono, with when the first of those was captured.
    pub fn push(&mut self, samples: &[f32], captured: Instant, mut call: impl FnMut(&[f32], Instant)) {
        for (n, frame) in samples.chunks_exact(self.channels).enumerate() {
            if self.filled == 0 {
                self.captured = captured + duration_of(n, self.rate);
            }
            self.chunk[self.filled] = self.mixdown.mix(frame);
            self.filled += 1;
            if self.filled == self.chunk.len() {
                self.filled = 0;
                self.mixdown.weigh(self.chunk.len());
                match &mut self.resampling {
                    Some(resampling) => {
                        process(&mut resampling.fft, &self.chunk, &mut resampling.frame);
                        call(&resampling.frame, self.captured - resampling.delay);
                    }
                    None => call(&self.chunk, self.captured),
                }
            }
        }
    }
}

impl Mixdown {
    fn new(channels: usize) -> Self {
        Self {
            weights: vec![1.0 / channels as f32; channels].into(),
            levels: vec![0.0; channels].into(),
            energies: vec![0.0; channels].into(),
        }
    }

    fn mix(&mut self, frame: &[f32]) -> f32 {
        // A faulty driver's NaN, infinite or huge sample would turn a level, and with it every
        // weight, to NaN for good: clip to full scale, as an integer device would, and take NaN
        // for silence.
        let clip = |sample: f32| if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) };
        self.energies.iter_mut().zip(frame).for_each(|(energy, sample)| *energy += clip(*sample).powi(2));
        frame.iter().zip(&self.weights).map(|(sample, weight)| clip(*sample) * weight).sum()
    }

    /// Weighs the channels anew after a chunk of `frames`, in proportion to their RMS levels.
    fn weigh(&mut self, frames: usize) {
        for (level, energy) in self.levels.iter_mut().zip(&mut self.energies) {
            *level += (*energy / frames as f32 - *level) * LEVEL_SMOOTHING;
            *energy = 0.0;
        }
        let total: f32 = self.levels.iter().map(|level| level.sqrt()).sum();
        // All silent, they keep their weights.
        if total > 0.0 {
            self.weights.iter_mut().zip(&self.levels).for_each(|(weight, level)| *weight = level.sqrt() / total);
        }
    }
}

impl Output {
    pub fn new(device: Format) -> Result<Self, Error> {
        let chunk = chunk_len(device)?;
        let resampling = Resampling::new(SAMPLE_RATE, device.rate)?;
        Ok(Self { channels: device.channels, rate: device.rate, chunk: vec![0.0; chunk].into(), played: chunk, resampling })
    }

    /// Fills the device's interleaved samples, the first of which plays at `plays`, with what
    /// `call` fills in at 48 kHz mono, told when the first of those plays.
    pub fn pull(&mut self, samples: &mut [f32], plays: Instant, mut call: impl FnMut(&mut [f32], Instant)) {
        for (n, frame) in samples.chunks_exact_mut(self.channels).enumerate() {
            if self.played == self.chunk.len() {
                self.played = 0;
                let plays = plays + duration_of(n, self.rate);
                match &mut self.resampling {
                    Some(resampling) => {
                        call(&mut resampling.frame, plays + resampling.delay);
                        process(&mut resampling.fft, &resampling.frame, &mut self.chunk);
                    }
                    None => call(&mut self.chunk, plays),
                }
            }
            frame.fill(self.chunk[self.played]);
            self.played += 1;
        }
    }
}

impl Resampling {
    /// None if the rates are the same.
    fn new(from: u32, to: u32) -> Result<Option<Self>, Error> {
        if from == to {
            return Ok(None);
        }
        let chunk_in = from as usize / 100;
        let fft = Fft::new(from as usize, to as usize, chunk_in, 1, FixedSync::Both)?;
        // 10 ms at either rate is a whole multiple of the rates' smallest common step.
        assert_eq!((fft.input_frames_next(), fft.output_frames_next()), (chunk_in, to as usize / 100), "the chunks are 10 ms");
        let delay = duration_of(fft.output_delay(), to);
        Ok(Some(Self { fft, frame: vec![0.0; FRAME_SAMPLES].into(), delay }))
    }
}

fn process(fft: &mut Fft<f32>, from: &[f32], to: &mut [f32]) {
    let input = SequentialSlice::new(from, 1, from.len()).expect("a mono slice holds its length");
    let frames = to.len();
    let mut output = SequentialSlice::new_mut(to, 1, frames).expect("a mono slice holds its length");
    fft.process_into_buffer(&input, &mut output, None).expect("the chunks have the sizes the resampler was made for");
}

/// How long `samples` take at `rate`.
fn duration_of(samples: usize, rate: u32) -> Duration {
    Duration::from_nanos(samples as u64 * 1_000_000_000 / u64::from(rate))
}

/// Samples in 10 ms at the device's rate.
fn chunk_len(device: Format) -> Result<usize, Error> {
    if device.channels == 0 {
        return Err(Error::NoChannels);
    }
    if device.rate == 0 || !device.rate.is_multiple_of(100) {
        return Err(Error::Rate(device.rate));
    }
    Ok(device.rate as usize / 100)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    fn sine(rate: u32, seconds: f32, frequency: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize).map(|n| 0.5 * (TAU * frequency * n as f32 / rate as f32).sin()).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The frequency of a sine sampled at `rate`, by its zero crossings.
    fn frequency(samples: &[f32], rate: u32) -> f32 {
        let crossings = samples.windows(2).filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0)).count();
        crossings as f32 / 2.0 / (samples.len() as f32 / rate as f32)
    }

    /// The middle half, past the resamplers' delay.
    fn settled(samples: &[f32]) -> &[f32] {
        &samples[samples.len() / 4..samples.len() * 3 / 4]
    }

    /// Through an input device's format into the call, and from there out of an output
    /// device's, in buffers of odd lengths; what the call got, and what the device played.
    fn round_trip(device: Format) -> (Vec<f32>, Vec<f32>) {
        let mut input = Input::new(device).unwrap();
        let mut output = Output::new(device).unwrap();
        let captured: Vec<f32> = sine(device.rate, 1.0, 440.0).iter().flat_map(|&sample| vec![sample; device.channels]).collect();

        let mut call = Vec::new();
        for buffer in captured.chunks(device.channels * 333) {
            input.push(buffer, Instant::now(), |frame, _| call.extend_from_slice(frame));
        }
        assert_eq!(call.len(), SAMPLE_RATE as usize, "one second at 48 kHz");

        let mut sent = call.iter();
        let mut played = vec![0.0; captured.len()];
        for buffer in played.chunks_mut(device.channels * 333) {
            output
                .pull(buffer, Instant::now(), |frame, _| frame.iter_mut().for_each(|sample| *sample = sent.next().copied().unwrap_or(0.0)));
        }
        (call, played)
    }

    #[test]
    fn a_48_khz_mono_device_passes_samples_unchanged() {
        let device = Format { channels: 1, rate: SAMPLE_RATE };
        let (_, played) = round_trip(device);
        assert_eq!(played, sine(SAMPLE_RATE, 1.0, 440.0));
    }

    #[test]
    fn stereo_is_mixed_down_and_copied_out() {
        let mut input = Input::new(Format { channels: 2, rate: SAMPLE_RATE }).unwrap();
        let mut call = Vec::new();
        input.push(&[0.3, 0.3].repeat(FRAME_SAMPLES), Instant::now(), |frame, _| call.extend_from_slice(frame));
        assert!(call.iter().all(|&sample| (sample - 0.3).abs() < 1e-6));

        let mut output = Output::new(Format { channels: 2, rate: SAMPLE_RATE }).unwrap();
        let mut played = [0.0; 6];
        output.pull(&mut played, Instant::now(), |frame, _| frame.iter_mut().enumerate().for_each(|(n, sample)| *sample = n as f32));
        assert_eq!(played, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn a_microphone_on_one_input_keeps_its_level_beside_a_silent_other() {
        let mut input = Input::new(Format { channels: 2, rate: SAMPLE_RATE }).unwrap();
        let captured: Vec<f32> = sine(SAMPLE_RATE, 1.0, 440.0).iter().flat_map(|&sample| [0.0, sample]).collect();
        let mut call = Vec::new();
        input.push(&captured, Instant::now(), |frame, _| call.extend_from_slice(frame));
        // Averaged for the first 10 ms, until the levels are known.
        let rms = rms(&call[FRAME_SAMPLES..]);
        assert!((0.35..0.36).contains(&rms), "rms {rms}");
    }

    #[test]
    fn a_broken_sample_leaves_the_microphone_heard() {
        let mut input = Input::new(Format { channels: 2, rate: SAMPLE_RATE }).unwrap();
        let mut captured: Vec<f32> = sine(SAMPLE_RATE, 1.0, 440.0).iter().flat_map(|&sample| [0.0, sample]).collect();
        captured[1] = f32::INFINITY;
        captured[3] = f32::NAN;
        let mut call = Vec::new();
        input.push(&captured, Instant::now(), |frame, _| call.extend_from_slice(frame));
        let rms = rms(&call[FRAME_SAMPLES..]);
        assert!((0.35..0.36).contains(&rms), "rms {rms}");
    }

    #[test]
    fn other_rates_are_converted_to_48_khz_and_back() {
        for rate in [44_100, 16_000, 96_000] {
            let (call, played) = round_trip(Format { channels: 2, rate });
            let left: Vec<f32> = played.iter().step_by(2).copied().collect();
            // The sine keeps its pitch in the call and back on the device, and its loudness.
            for (samples, at) in [(&call, SAMPLE_RATE), (&left, rate)] {
                let frequency = frequency(settled(samples), at);
                assert!((438.0..442.0).contains(&frequency), "{rate} Hz: a frequency of {frequency} Hz at {at} Hz");
            }
            let rms = rms(settled(&played));
            assert!((0.33..0.38).contains(&rms), "{rate} Hz: rms {rms}");
        }
    }

    #[test]
    fn the_call_is_told_when_its_frames_were_captured_and_play() {
        for rate in [SAMPLE_RATE, 44_100] {
            let device = Format { channels: 2, rate };
            let (mut input, mut output) = (Input::new(device).unwrap(), Output::new(device).unwrap());
            let delay = |resampling: &Option<Resampling>| resampling.as_ref().map_or(Duration::ZERO, |resampling| resampling.delay);
            let (captured_delay, played_delay) = (delay(&input.resampling), delay(&output.resampling));
            let start = Instant::now();
            let mut samples = vec![0.0; 2 * 333];
            let (mut captured, mut played) = (Vec::new(), Vec::new());
            for buffer in 0..100 {
                let at = start + duration_of(buffer * 333, rate);
                input.push(&samples, at, |_, captured_at| captured.push(captured_at));
                output.pull(&mut samples, at, |_, plays| played.push(plays));
            }
            for (frame, (&captured, &plays)) in captured.iter().zip(&played).enumerate() {
                let at = start + frame as u32 * Duration::from_millis(10);
                let off = |told: Instant, at: Instant| told.max(at) - told.min(at);
                // Within what rounding the samples' times to nanoseconds loses.
                assert!(off(captured + captured_delay, at) <= Duration::from_micros(1), "{rate} Hz: frame {frame} captured off");
                assert!(off(plays, at + played_delay) <= Duration::from_micros(1), "{rate} Hz: frame {frame} plays off");
            }
            // The output asks for a chunk as it starts playing it, the input hands one over once
            // it is full.
            let chunks = 100 * 333 / (rate as usize / 100);
            assert_eq!((captured.len(), played.len()), (chunks, chunks + 1), "{rate} Hz");
        }
    }

    #[test]
    fn formats_without_10_ms_chunks_or_channels_are_refused() {
        assert!(matches!(Input::new(Format { channels: 1, rate: 22_050 }), Err(Error::Rate(22_050))));
        assert!(matches!(Output::new(Format { channels: 0, rate: SAMPLE_RATE }), Err(Error::NoChannels)));
    }
}
