//! Signals for tests, at 48 kHz in −1.0..=1.0, and packets of them as they arrive.

use std::f32::consts::{PI, TAU};

use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES, Level, SAMPLE_RATE};
use opus::{Application, Bitrate, Channels, Encoder};

use crate::jitter::Packet;

const RATE: f32 = SAMPLE_RATE as f32;

/// A synthetic voice that RNNoise takes for speech: a 120 Hz pulse train with vibrato, shaped
/// by the formants of an "a" and spoken in four syllables a second, peaking at `peak`.
pub fn voice(samples: usize, peak: f32) -> Vec<f32> {
    // Frequency and bandwidth of the first three formants of "a", in Hz.
    const FORMANTS: [(f32, f32); 3] = [(700.0, 130.0), (1220.0, 70.0), (2600.0, 160.0)];
    let mut resonators = [(0.0f32, 0.0f32); 3];
    let mut phase = 0.0;
    let voice: Vec<f32> = (0..samples)
        .map(|n| {
            let t = n as f32 / RATE;
            phase += (120.0 + 8.0 * (TAU * 5.0 * t).sin()) / RATE;
            let pulse = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            let shaped: f32 = FORMANTS
                .iter()
                .zip(&mut resonators)
                .map(|(&(frequency, bandwidth), (last, before))| {
                    let radius = (-PI * bandwidth / RATE).exp();
                    let output = pulse * (1.0 - radius) + 2.0 * radius * (TAU * frequency / RATE).cos() * *last - radius * radius * *before;
                    (*last, *before) = (output, *last);
                    output
                })
                .sum();
            shaped * (0.5 - 0.5 * (TAU * 4.0 * t).cos()).sqrt()
        })
        .collect();
    let loudest = voice.iter().fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
    voice.iter().map(|sample| sample / loudest * peak).collect()
}

/// White noise of uniform amplitude up to `peak`, the same on every run.
pub fn noise(samples: usize, peak: f32) -> Vec<f32> {
    noise_from(0x2545_f491, samples, peak)
}

/// Another [`noise`], unrelated to it, from the nonzero `seed`.
pub fn noise_from(seed: u32, samples: usize, peak: f32) -> Vec<f32> {
    let mut state = seed;
    (0..samples)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * peak
        })
        .collect()
}

/// A talkspurt of a sine as the sender would encode it, its frames arriving 10 ms apart.
pub fn talkspurt(frames: u32, frequency: f32, amplitude: f32) -> Vec<Packet> {
    let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).unwrap();
    encoder.set_bitrate(Bitrate::Bits(48_000)).unwrap();
    encoder.set_inband_fec(true).unwrap();
    encoder.set_packet_loss_perc(10).unwrap();
    (0..frames)
        .map(|index| {
            let samples: Vec<f32> = (0..FRAME_SAMPLES)
                .map(|n| amplitude * (TAU * frequency * (index as usize * FRAME_SAMPLES + n) as f32 / RATE).sin())
                .collect();
            let opus = encoder.encode_vec_float(&samples, 320).unwrap();
            let arrival_us = u64::from(index) * FRAME_DURATION.as_micros() as u64;
            Packet::new(index, index + 1 == frames, Level::of(&samples), arrival_us, &opus).unwrap()
        })
        .collect()
}
