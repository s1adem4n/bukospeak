use std::time::Duration;

use bukospeak_proto::audio::{FRAME_DURATION, Level, SAMPLE_RATE};

use crate::jitter::{JitterBuffer, Packet, Playout};

const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

const FRAME_US: f64 = FRAME_DURATION.as_micros() as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Played {
    Silent,
    Frame(u32),
    Fec(u32),
    Conceal,
}

pub struct Simulation {
    pub drift_ppm: f64,
    pub base_delay_us: u64,
    pub jitter_us: u64,
    pub lost: fn(u32) -> bool,
    pub level: fn(u32) -> Level,
}

impl Default for Simulation {
    fn default() -> Self {
        Self { drift_ppm: 0.0, base_delay_us: 20_000, jitter_us: 0, lost: |_| false, level: |_| Level::new(20) }
    }
}

impl Simulation {
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
    fn from(playout: Playout<'_>) -> Self {
        match playout {
            Playout::Silent => Self::Silent,
            Playout::Frame(opus) => Self::Frame(index_of(opus)),
            Playout::Fec(opus) => Self::Fec(index_of(opus).wrapping_sub(1)),
            Playout::Conceal => Self::Conceal,
        }
    }
}

pub fn heard(played: &[Played]) -> Vec<u32> {
    played
        .iter()
        .filter_map(|played| match *played {
            Played::Frame(index) | Played::Fec(index) => Some(index),
            Played::Silent | Played::Conceal => None,
        })
        .collect()
}

#[derive(Clone, Copy)]
pub struct Room {
    pub output_drift_ppm: f64,
    pub input_drift_ppm: f64,
    pub output_buffer: usize,
    pub input_buffer: usize,
    pub output_latency_us: u64,
    pub input_latency_us: u64,
    pub echo: &'static [(u64, f32)],
    pub callback_jitter_us: u64,
    pub timestamp_jitter_us: u64,
    pub input_lost: &'static [(usize, usize)],
    pub output_stalls: &'static [(usize, usize)],
    pub seed: u64,
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
            timestamp_jitter_us: 200,
            input_lost: &[],
            output_stalls: &[],
            seed: SEED,
        }
    }
}

impl Room {
    pub fn run(
        &self,
        near: &[f32],
        duration_us: u64,
        mut play: impl FnMut(Duration, &mut [f32]),
        mut capture: impl FnMut(Duration, &[f32]),
    ) {
        let mut random = self.seed;
        let mut uniform = move |range_us: u64| {
            if range_us == 0 { 0.0 } else { (next_random(&mut random) % (range_us + 1)) as f64 / 1e6 }
        };
        let output_rate = f64::from(SAMPLE_RATE) * (1.0 + self.output_drift_ppm / 1e6);
        let input_rate = f64::from(SAMPLE_RATE) * (1.0 + self.input_drift_ppm / 1e6);
        let duration = duration_us as f64 / 1e6;
        let (output_latency, input_latency) = (self.output_latency_us as f64 / 1e6, self.input_latency_us as f64 / 1e6);
        let told = |at: f64, noise: f64| Duration::from_secs_f64((at + noise).max(0.0));
        let mut played: Vec<f32> = Vec::new();
        let mut captured = vec![0.0; self.input_buffer];
        let (mut outputs, mut inputs) = (0usize, 0usize);
        let (mut output_late, mut input_late) = (uniform(self.callback_jitter_us), uniform(self.callback_jitter_us));
        loop {
            let output_start = outputs * self.output_buffer + self.silence_through(outputs);
            let output_plays = output_start as f64 / output_rate + output_latency;
            let output_runs = output_plays - output_latency + output_late;
            let input_captured = (inputs * self.input_buffer) as f64 / input_rate;
            let input_runs = ((inputs + 1) * self.input_buffer) as f64 / input_rate + input_latency + input_late;
            if output_runs.min(input_runs) >= duration {
                return;
            }
            if output_runs <= input_runs {
                played.resize(output_start + self.output_buffer, 0.0);
                let noise = uniform(2 * self.timestamp_jitter_us) - self.timestamp_jitter_us as f64 / 1e6;
                play(told(output_plays, noise), &mut played[output_start..]);
                outputs += 1;
                output_late = uniform(self.callback_jitter_us);
            } else {
                let lost = self.input_lost.iter().any(|&(first, count)| (first..first + count).contains(&inputs));
                if !lost {
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
                    let noise = uniform(2 * self.timestamp_jitter_us) - self.timestamp_jitter_us as f64 / 1e6;
                    capture(told(input_captured, noise), &captured);
                }
                inputs += 1;
                input_late = uniform(self.callback_jitter_us);
            }
        }
    }

    fn silence_through(&self, callback: usize) -> usize {
        self.output_stalls.iter().filter(|&&(stalled, _)| stalled <= callback).map(|&(_, buffers)| buffers * self.output_buffer).sum()
    }
}

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

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
