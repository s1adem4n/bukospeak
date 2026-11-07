use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES, SAMPLE_RATE};
use rtrb::RingBuffer;

use crate::capture::Capture;
use crate::conditioner::Conditioning;
use crate::jitter::Packet;
use crate::microphone::{Microphone, Settings};
use crate::playout::Mixer;
use crate::suppression::{Stage, Suppression};

const TAIL_FRAMES: usize = 10;

#[derive(Clone, Debug)]
pub struct Options {
    pub conditioning: Conditioning,
    pub suppression: Suppression,
}

pub fn play(recording: &[f32], options: Options) -> Result<Vec<f32>, opus::Error> {
    let settings = Arc::new(Settings::default());
    let (frames, mut captured) = RingBuffer::new(2);
    let (mut microphone, _tap) = Microphone::new(options.conditioning, frames, thread::current(), settings);
    let mut capture = Capture::new(options.conditioning, Stage::new(&options.suppression))?;
    let mut mixer = Mixer::new()?;
    let sender = MemberId(1);
    let start = Instant::now();
    let frames = recording.as_chunks::<FRAME_SAMPLES>().0;
    let mut played = Vec::with_capacity((frames.len() + TAIL_FRAMES) * FRAME_SAMPLES);
    let mut mix = [0.0; FRAME_SAMPLES];
    for tick in 0..frames.len() + TAIL_FRAMES {
        if let Some(frame) = frames.get(tick) {
            microphone.push(frame, start + FRAME_DURATION * tick as u32);
            let next = captured.pop().expect("each frame pushed is handed over");
            let arrival_us = (FRAME_DURATION * tick as u32).as_micros() as u64;
            capture.take(&next, Instant::now(), |encoded| {
                let packet = Packet::new(encoded.index, encoded.last, encoded.level, arrival_us, encoded.opus())
                    .expect("an encoded frame fits a packet");
                mixer.insert(sender, packet);
            })?;
        }
        mixer.render(&mut mix, 1.0);
        played.extend_from_slice(&mix);
    }
    Ok(played)
}

pub fn delay(recording: &[f32], played: &[f32], most: Duration) -> Duration {
    let most = (most.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize;
    let correlation = |lag: usize| -> f64 {
        recording.iter().zip(played.iter().skip(lag)).map(|(captured, heard)| f64::from(*captured) * f64::from(*heard)).sum()
    };
    let lag = (0..=most).max_by(|a, b| correlation(*a).total_cmp(&correlation(*b))).unwrap_or(0);
    Duration::from_secs_f64(lag as f64 / f64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use SAMPLE_RATE;

    use super::*;
    use crate::signal::voice;

    #[test]
    fn a_voice_is_heard_after_the_playout_delay() {
        let second = SAMPLE_RATE as usize;
        let recording: Vec<f32> = [vec![0.0; second / 2], voice(3 * second, 0.3)].concat();
        let options = Options { conditioning: Conditioning::Platform, suppression: Suppression::Off };
        let played = play(&recording, options).unwrap();
        assert_eq!(played.len(), recording.len() + TAIL_FRAMES * FRAME_SAMPLES);
        let delay = delay(&recording[5 * second / 2..], &played[5 * second / 2..], Duration::from_millis(100));
        assert!((Duration::from_millis(20)..Duration::from_millis(50)).contains(&delay), "delay {delay:?}");
    }
}
