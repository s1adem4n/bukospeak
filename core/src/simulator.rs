//! Deterministic worlds for tests. A network: a sender on a drifting clock, delay with jitter,
//! losses, and a receiver that plays a frame every 10 ms on its own clock. And a room: a speaker
//! and a microphone on two devices with their own clocks and buffer sizes, the speaker's sound
//! reaching the microphone as echo.

use std::time::Duration;

use bukospeak_proto::audio::{FRAME_DURATION, Level, SAMPLE_RATE};

use crate::jitter::{JitterBuffer, Packet, Playout};

/// Seeds the jitter, so every run has the same.
const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

const FRAME_US: f64 = FRAME_DURATION.as_micros() as f64;

/// What the receiver played in one 10 ms step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Played {
    Silent,
    Frame(u32),
    Fec(u32),
    Conceal,
}

pub struct Simulation {
    /// The sender's clock runs this many parts per million fast (or slow, if negative).
    pub drift_ppm: f64,
    pub base_delay_us: u64,
    /// Each frame's delay grows by up to this much, uniformly.
    pub jitter_us: u64,
    /// Whether the frame with this index is lost.
    pub lost: fn(u32) -> bool,
    /// The frames' levels, by index.
    pub level: fn(u32) -> Level,
}

impl Default for Simulation {
    fn default() -> Self {
        Self { drift_ppm: 0.0, base_delay_us: 20_000, jitter_us: 0, lost: |_| false, level: |_| Level::new(20) }
    }
}

impl Simulation {
    /// Sends the frames with these indices, in order: a run of consecutive ones is a talkspurt,
    /// whose last frame is flagged. Plays until `duration_us` on the receiver's clock.
    pub fn run(&self, sent: impl IntoIterator<Item = u32>, duration_us: u64) -> (Vec<Played>, JitterBuffer) {
        let mut sent = sent.into_iter().peekable();
        let mut random = SEED;
        let mut arrivals = Vec::new();
        while let Some(index) = sent.next() {
            let last = sent.peek() != Some(&index.wrapping_add(1));
            if (self.lost)(index) {
                continue;
            }
            let sent_us = f64::from(index) * FRAME_US / (1.0 + self.drift_ppm / 1e6);
            let jitter = if self.jitter_us == 0 { 0 } else { next_random(&mut random) % self.jitter_us };
            let arrival_us = sent_us as u64 + self.base_delay_us + jitter;
            arrivals.push(Packet::new(index, last, (self.level)(index), arrival_us, &index.to_be_bytes()).expect("4 bytes fit"));
        }
        arrivals.sort_by_key(|packet| packet.arrival_us);

        let mut buffer = JitterBuffer::default();
        let mut arrivals = arrivals.into_iter().peekable();
        let mut played = Vec::new();
        for tick in 0..duration_us / FRAME_US as u64 {
            let now_us = tick * FRAME_US as u64;
            while let Some(packet) = arrivals.next_if(|packet| packet.arrival_us <= now_us) {
                buffer.insert(packet);
            }
            played.push(Played::from(buffer.pop()));
        }
        (played, buffer)
    }
}

impl From<Playout<'_>> for Played {
    /// The packets in a simulation hold their frame's index.
    fn from(playout: Playout<'_>) -> Self {
        match playout {
            Playout::Silent => Self::Silent,
            Playout::Frame(opus) => Self::Frame(index_of(opus)),
            Playout::Fec(opus) => Self::Fec(index_of(opus).wrapping_sub(1)),
            Playout::Conceal => Self::Conceal,
        }
    }
}

/// Which frames played or were recovered, in order.
pub fn heard(played: &[Played]) -> Vec<u32> {
    played
        .iter()
        .filter_map(|played| match *played {
            Played::Frame(index) | Played::Fec(index) => Some(index),
            Played::Silent | Played::Conceal => None,
        })
        .collect()
}

/// A speaker and a microphone in a room, each on a device of its own, at 48 kHz on its clock.
pub struct Room {
    /// The output device's clock runs this many parts per million fast, or slow if negative.
    pub output_drift_ppm: f64,
    pub input_drift_ppm: f64,
    /// The samples the output device asks for at once, and the input device hands over.
    pub output_buffer: usize,
    pub input_buffer: usize,
    /// From asking for samples to playing them, and from capturing samples to handing them over.
    pub output_latency_us: u64,
    pub input_latency_us: u64,
    /// The ways the speaker's sound reaches the microphone: after how long, and how loud.
    pub echo: &'static [(u64, f32)],
    /// Each callback runs late by up to this much, uniformly.
    pub callback_jitter_us: u64,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            output_drift_ppm: 0.0,
            input_drift_ppm: 0.0,
            output_buffer: 1024,
            input_buffer: 441,
            output_latency_us: 30_000,
            input_latency_us: 10_000,
            echo: &[(2_000, 0.5), (9_000, 0.15)],
            callback_jitter_us: 3_000,
        }
    }
}

impl Room {
    /// Runs both devices for `duration_us`, in the order of their callbacks: the speaker plays
    /// what `play` fills, and the microphone hands `capture` `near` and the speaker's echo. Each
    /// is told when its callback runs.
    pub fn run(
        &self,
        near: &[f32],
        duration_us: u64,
        mut play: impl FnMut(Duration, &mut [f32]),
        mut capture: impl FnMut(Duration, &[f32]),
    ) {
        let mut random = SEED;
        let mut late =
            move || if self.callback_jitter_us == 0 { 0.0 } else { (next_random(&mut random) % self.callback_jitter_us) as f64 / 1e6 };
        let output_rate = f64::from(SAMPLE_RATE) * (1.0 + self.output_drift_ppm / 1e6);
        let input_rate = f64::from(SAMPLE_RATE) * (1.0 + self.input_drift_ppm / 1e6);
        let duration = duration_us as f64 / 1e6;
        let (output_latency, input_latency) = (self.output_latency_us as f64 / 1e6, self.input_latency_us as f64 / 1e6);
        let mut played: Vec<f32> = Vec::new();
        let mut captured = vec![0.0; self.input_buffer];
        let (mut outputs, mut inputs) = (0usize, 0usize);
        loop {
            // The output device asks for its next buffer once it starts playing the one before.
            let output_at = (outputs * self.output_buffer) as f64 / output_rate;
            // The input device hands over a buffer once its last sample is captured.
            let input_at = ((inputs + 1) * self.input_buffer) as f64 / input_rate + input_latency;
            if output_at.min(input_at) >= duration {
                return;
            }
            if output_at <= input_at {
                let start = played.len();
                played.resize(start + self.output_buffer, 0.0);
                play(Duration::from_secs_f64(output_at + late()), &mut played[start..]);
                outputs += 1;
            } else {
                for (offset, sample) in captured.iter_mut().enumerate() {
                    let index = inputs * self.input_buffer + offset;
                    let at = index as f64 / input_rate;
                    let echo: f32 = self
                        .echo
                        .iter()
                        .map(|&(delay_us, gain)| gain * sample_at(&played, (at - output_latency - delay_us as f64 / 1e6) * output_rate))
                        .sum();
                    *sample = near.get(index).copied().unwrap_or(0.0) + echo;
                }
                capture(Duration::from_secs_f64(input_at + late()), &captured);
                inputs += 1;
            }
        }
    }
}

/// A signal between its samples, linearly; silence before and after it.
fn sample_at(signal: &[f32], position: f64) -> f32 {
    if position < 0.0 {
        return 0.0;
    }
    let (index, fraction) = (position as usize, (position.fract()) as f32);
    let sample = |index: usize| signal.get(index).copied().unwrap_or(0.0);
    sample(index) + (sample(index + 1) - sample(index)) * fraction
}

fn index_of(opus: &[u8]) -> u32 {
    u32::from_be_bytes(opus.try_into().expect("the simulation sends 4-byte packets"))
}

/// xorshift64.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
