//! The captured voice, from microphone frames to the Opus frames the uplink sends.
//!
//! The capture callback owns it and hands it one 10 ms frame at a time. It suppresses noise
//! with RNNoise, lets the gate decide, and encodes every frame, sent or not, so the encoder
//! stays warm and the frame index counts the sender's whole timeline. Nothing here allocates
//! after construction.

use bukospeak_proto::audio::{FRAME_SAMPLES, Level, MAX_OPUS_LEN, SAMPLE_RATE};
use nnnoiseless::DenoiseState;
use opus::{Application, Bitrate, Channels, Encoder, Signal};

use crate::gate::{Decision, Gate};

/// Low enough that libopus stays in hybrid mode, where in-band FEC exists.
const BITRATE: i32 = 48_000;

/// The loss the encoder spends FEC on; it only adds FEC when told to expect loss.
const EXPECTED_LOSS_PERCENT: i32 = 10;

/// Frames sent ahead of the one that opens the gate: 50 ms, so word onsets survive.
const PREROLL: usize = 5;

/// RNNoise takes and returns samples scaled like 16-bit PCM.
const PCM_SCALE: f32 = 32_768.0;

const _: () = assert!(DenoiseState::FRAME_SIZE == FRAME_SAMPLES);
// An encoded frame's length is stored as a u16.
const _: () = assert!(MAX_OPUS_LEN <= u16::MAX as usize);

/// A frame to send: fixed size, so producing it never allocates.
#[derive(Clone, Copy, Debug)]
pub struct Encoded {
    pub index: u32,
    pub last: bool,
    pub level: Level,
    len: u16,
    opus: [u8; MAX_OPUS_LEN],
}

impl Encoded {
    pub fn opus(&self) -> &[u8] {
        &self.opus[..usize::from(self.len)]
    }
}

pub struct Capture {
    denoiser: Box<DenoiseState<'static>>,
    encoder: Encoder,
    gate: Gate,
    next_index: u32,
    /// The latest frames the gate kept back, oldest first.
    preroll: [Option<Encoded>; PREROLL],
    scaled: [f32; FRAME_SAMPLES],
    denoised: [f32; FRAME_SAMPLES],
}

impl Capture {
    pub fn new() -> Result<Self, opus::Error> {
        let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)?;
        encoder.set_bitrate(Bitrate::Bits(BITRATE))?;
        encoder.set_signal(Signal::Voice)?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(EXPECTED_LOSS_PERCENT)?;
        Ok(Self {
            denoiser: DenoiseState::new(),
            encoder,
            gate: Gate::default(),
            next_index: 0,
            preroll: [None; PREROLL],
            scaled: [0.0; FRAME_SAMPLES],
            denoised: [0.0; FRAME_SAMPLES],
        })
    }

    /// Takes a captured frame and hands `send` what the gate lets through: nothing, the frame,
    /// or, when speech starts, the pre-roll and then the frame.
    pub fn process(&mut self, input: &[f32; FRAME_SAMPLES], mut send: impl FnMut(&Encoded)) -> Result<(), opus::Error> {
        // A NaN, infinite or huge sample from a faulty driver would turn RNNoise's state to NaN for
        // good, and NaN in its spectrum is undefined behavior in its FFT: clip to full scale, as an
        // integer device would, and take NaN for silence.
        self.scaled.iter_mut().zip(input).for_each(|(scaled, sample)| {
            *scaled = if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) * PCM_SCALE };
        });
        let voice_probability = self.denoiser.process_frame(&mut self.denoised, &self.scaled);
        self.denoised.iter_mut().for_each(|sample| *sample /= PCM_SCALE);

        let level = Level::of(&self.denoised);
        let decision = self.gate.decide(voice_probability, level);
        let mut frame = Encoded { index: self.next_index, last: decision == Decision::Closes, level, len: 0, opus: [0; MAX_OPUS_LEN] };
        frame.len = self.encoder.encode_float(&self.denoised, &mut frame.opus)? as u16;
        self.next_index = self.next_index.wrapping_add(1);

        match decision {
            Decision::Quiet => {
                self.preroll.rotate_left(1);
                self.preroll[PREROLL - 1] = Some(frame);
            }
            Decision::Opens => {
                self.preroll.iter_mut().filter_map(Option::take).for_each(|frame| send(&frame));
                send(&frame);
            }
            Decision::Speaks | Decision::Closes => send(&frame),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate;
    use crate::signal::{noise, voice};

    const SECOND: usize = SAMPLE_RATE as usize;

    /// Captures a signal and returns the frames sent.
    fn capture(signal: &[f32]) -> Vec<Encoded> {
        let mut capture = Capture::new().unwrap();
        let mut sent = Vec::new();
        for frame in signal.as_chunks().0 {
            capture.process(frame, |encoded| sent.push(*encoded)).unwrap();
        }
        sent
    }

    #[test]
    fn speech_is_sent_with_its_preroll_and_ends_with_a_last_frame() {
        let signal: Vec<f32> = [vec![0.0; SECOND], voice(2 * SECOND, 0.3), vec![0.0; SECOND]].concat();
        let sent = capture(&signal);

        let indices: Vec<u32> = sent.iter().map(|frame| frame.index).collect();
        let first = indices[0];
        assert_eq!(indices, (first..first + sent.len() as u32).collect::<Vec<_>>(), "one talkspurt");
        // The voice starts at frame 100, after digital silence: the gate opens once the denoiser
        // settled, with the pre-roll before.
        let opens = 100 + gate::SETTLE;
        assert!((opens - PREROLL as u32..opens + 5).contains(&first), "starts at frame {first}");
        assert!(sent.len() >= 200, "only {} frames of 2 s of speech sent", sent.len());
        assert_eq!(sent.iter().filter(|frame| frame.last).count(), 1);
        assert!(sent.last().unwrap().last);
    }

    #[test]
    fn every_frame_is_hybrid_10_ms_so_it_carries_fec() {
        for frame in capture(&voice(2 * SECOND, 0.3)) {
            // RFC 6716 3.1: TOC configurations 12 to 15 are hybrid; 12 and 14 are 10 ms frames.
            let configuration = frame.opus()[0] >> 3;
            assert!(matches!(configuration, 12 | 14), "TOC configuration {configuration} in frame {}", frame.index);
        }
    }

    #[test]
    fn noise_and_silence_send_nothing() {
        let signal: Vec<f32> = [vec![0.0; SECOND], noise(3 * SECOND, 0.05), vec![0.0; SECOND]].concat();
        assert!(capture(&signal).is_empty());
    }

    #[test]
    fn speech_after_broken_samples_is_still_sent() {
        let mut signal = voice(2 * SECOND, 0.3);
        for (sample, broken) in signal.iter_mut().zip([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30]) {
            *sample = broken;
        }
        assert!(capture(&signal).len() >= 150);
    }

    #[test]
    fn levels_follow_the_denoised_signal() {
        let sent = capture(&voice(SECOND, 0.3));
        let loudest = sent.iter().map(|frame| frame.level).min().unwrap();
        assert!((10..30).contains(&loudest.minus_dbov()), "loudest frame at -{} dBov", loudest.minus_dbov());
    }
}
