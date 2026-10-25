//! A deterministic network for tests: a sender on a drifting clock, delay with jitter, losses,
//! and a receiver that plays a frame every 10 ms on its own clock.

use bukospeak_proto::audio::{FRAME_DURATION, Level};

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
