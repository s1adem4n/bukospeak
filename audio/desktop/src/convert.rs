//! Between a device's format and the call's 48 kHz mono: channels mixed down or copied out, and
//! the rate converted in 10 ms chunks when the device runs at another. Nothing here allocates
//! after construction.
//!
//! The input and the output device each run on their own clock; the few ppm between them and
//! the senders' clocks are the jitter buffers' to absorb, so the ratio here is fixed.

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

/// From the input device's samples to the call's.
pub struct Input {
    channels: usize,
    /// 10 ms at the device's rate, mixed down to mono.
    chunk: Box<[f32]>,
    filled: usize,
    resampling: Option<Resampling>,
}

/// From the call's samples to the output device's.
pub struct Output {
    channels: usize,
    /// 10 ms at the device's rate, in mono.
    chunk: Box<[f32]>,
    played: usize,
    resampling: Option<Resampling>,
}

/// A fixed-ratio resampler between 10 ms at the device's rate and a frame at 48 kHz.
struct Resampling {
    fft: Fft<f32>,
    frame: Box<[f32]>,
}

impl Input {
    pub fn new(device: Format) -> Result<Self, Error> {
        let chunk = chunk_len(device)?;
        let resampling = Resampling::new(device.rate, SAMPLE_RATE)?;
        Ok(Self { channels: device.channels, chunk: vec![0.0; chunk].into(), filled: 0, resampling })
    }

    /// Takes the device's interleaved samples and hands `call` them at 48 kHz mono.
    pub fn push(&mut self, samples: &[f32], mut call: impl FnMut(&[f32])) {
        for frame in samples.chunks_exact(self.channels) {
            self.chunk[self.filled] = frame.iter().sum::<f32>() / self.channels as f32;
            self.filled += 1;
            if self.filled == self.chunk.len() {
                self.filled = 0;
                match &mut self.resampling {
                    Some(resampling) => {
                        process(&mut resampling.fft, &self.chunk, &mut resampling.frame);
                        call(&resampling.frame);
                    }
                    None => call(&self.chunk),
                }
            }
        }
    }
}

impl Output {
    pub fn new(device: Format) -> Result<Self, Error> {
        let chunk = chunk_len(device)?;
        let resampling = Resampling::new(SAMPLE_RATE, device.rate)?;
        Ok(Self { channels: device.channels, chunk: vec![0.0; chunk].into(), played: chunk, resampling })
    }

    /// Fills the device's interleaved samples with what `call` fills in at 48 kHz mono.
    pub fn pull(&mut self, samples: &mut [f32], mut call: impl FnMut(&mut [f32])) {
        for frame in samples.chunks_exact_mut(self.channels) {
            if self.played == self.chunk.len() {
                self.played = 0;
                match &mut self.resampling {
                    Some(resampling) => {
                        call(&mut resampling.frame);
                        process(&mut resampling.fft, &resampling.frame, &mut self.chunk);
                    }
                    None => call(&mut self.chunk),
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
        Ok(Some(Self { fft, frame: vec![0.0; FRAME_SAMPLES].into() }))
    }
}

fn process(fft: &mut Fft<f32>, from: &[f32], to: &mut [f32]) {
    let input = SequentialSlice::new(from, 1, from.len()).expect("a mono slice holds its length");
    let frames = to.len();
    let mut output = SequentialSlice::new_mut(to, 1, frames).expect("a mono slice holds its length");
    fft.process_into_buffer(&input, &mut output, None).expect("the chunks have the sizes the resampler was made for");
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
            input.push(buffer, |frame| call.extend_from_slice(frame));
        }
        assert_eq!(call.len(), SAMPLE_RATE as usize, "one second at 48 kHz");

        let mut sent = call.iter();
        let mut played = vec![0.0; captured.len()];
        for buffer in played.chunks_mut(device.channels * 333) {
            output.pull(buffer, |frame| frame.iter_mut().for_each(|sample| *sample = sent.next().copied().unwrap_or(0.0)));
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
        input.push(&[0.2, 0.4].repeat(FRAME_SAMPLES), |frame| call.extend_from_slice(frame));
        assert!(call.iter().all(|&sample| (sample - 0.3).abs() < 1e-6));

        let mut output = Output::new(Format { channels: 2, rate: SAMPLE_RATE }).unwrap();
        let mut played = [0.0; 6];
        output.pull(&mut played, |frame| frame.iter_mut().enumerate().for_each(|(n, sample)| *sample = n as f32));
        assert_eq!(played, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
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
    fn formats_without_10_ms_chunks_or_channels_are_refused() {
        assert!(matches!(Input::new(Format { channels: 1, rate: 22_050 }), Err(Error::Rate(22_050))));
        assert!(matches!(Output::new(Format { channels: 0, rate: SAMPLE_RATE }), Err(Error::NoChannels)));
    }
}
