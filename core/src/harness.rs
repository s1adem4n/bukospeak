//! A recording through the capture the apps send with and the mix a listener plays it with, for
//! measuring and hearing what the processing does, outside a call.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES, SAMPLE_RATE};
use rtrb::RingBuffer;

use crate::capture::Capture;
pub use crate::capture::Suppression;
use crate::conditioner::Conditioning;
use crate::jitter::Packet;
use crate::microphone::{Microphone, Settings};
use crate::playout::Mixer;

/// Frames played after the recording ended, so the listener's buffer drains.
const TAIL_FRAMES: usize = 10;

/// How the capture processes the recording.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub conditioning: Conditioning,
    pub suppression: Suppression,
}

/// Plays `recording`, 48 kHz mono, as a listener without loss and jitter hears it: what the
/// gate held back is silence, and the playout delay comes first.
pub fn play(recording: &[f32], options: Options) -> Result<Vec<f32>, opus::Error> {
    let settings = Arc::new(Settings::default());
    let noise_suppression = options.suppression != Suppression::Off;
    settings.noise_suppression.store(noise_suppression, Ordering::Relaxed);
    // A frame and the slot kept for a stop.
    let (frames, mut captured) = RingBuffer::new(2);
    // Nothing plays, which AEC3 hears as silence.
    let (mut microphone, _tap) = Microphone::new(options.conditioning, frames, thread::current(), settings);
    let mut capture = Capture::new(options.conditioning)?;
    #[cfg(feature = "deepfilter")]
    if options.suppression == Suppression::DeepFilter {
        capture = capture.with_deep_filter(crate::deep_filter::DeepFilter::new().expect("the built-in model builds"));
    }
    capture.set_noise_suppression(noise_suppression);
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
        mixer.render(&mut mix);
        played.extend_from_slice(&mix);
    }
    Ok(played)
}

/// How much later `played` carries `recording` than it was captured: the lag that correlates
/// them best, up to `most`. It takes time in the product of their lengths, so it suits excerpts
/// of seconds.
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
        // Once the listener's buffer dropped the pre-roll's frames it held beyond its target.
        let delay = delay(&recording[5 * second / 2..], &played[5 * second / 2..], Duration::from_millis(100));
        assert!((Duration::from_millis(20)..Duration::from_millis(50)).contains(&delay), "delay {delay:?}");
    }
}
