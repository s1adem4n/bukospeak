//! The captured voice, from microphone frames to the Opus frames the uplink sends.
//!
//! The capture callback owns it and hands it one 10 ms frame at a time. It suppresses noise
//! with RNNoise, conditions the signal where the platform doesn't, lets the gate decide, and
//! encodes every frame, sent or not, so the encoder stays warm and the frame index counts the
//! sender's whole timeline. Nothing here allocates after construction but the first frame on
//! each thread, where RNNoise's FFT sets up its plans and scratch buffers in thread-locals.

use std::time::Instant;

use bukospeak_proto::audio::{FRAME_SAMPLES, Level, MAX_OPUS_LEN, SAMPLE_RATE};
use nnnoiseless::DenoiseState;
use opus::{Application, Bitrate, Channels, Encoder, Signal};

use crate::conditioner::{Conditioner, Conditioning};
use crate::echo::Tap;
use crate::gate::{Decision, Gate};
use crate::limiter::limit;

/// Low enough that libopus stays in hybrid mode, where in-band FEC exists.
const BITRATE: i32 = 48_000;

/// The loss the encoder spends FEC on; it only adds FEC when told to expect loss.
const EXPECTED_LOSS_PERCENT: i32 = 10;

/// Frames sent ahead of the one that opens the gate: 50 ms, so word onsets survive.
const PREROLL: usize = 5;

/// RNNoise takes and returns samples scaled like 16-bit PCM.
pub(crate) const PCM_SCALE: f32 = 32_768.0;

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
    /// None where the platform's audio conditions the signal.
    conditioner: Option<Conditioner>,
    encoder: Encoder,
    gate: Gate,
    noise_suppression: bool,
    next_index: u32,
    /// The latest frames the gate kept back, oldest first.
    preroll: [Option<Encoded>; PREROLL],
    scaled: [f32; FRAME_SAMPLES],
    denoised: [f32; FRAME_SAMPLES],
}

impl Capture {
    /// Makes the capture, and where the engine conditions the signal, the speaker's end of what
    /// it played, whose echo the capture cancels.
    pub fn new(conditioning: Conditioning) -> Result<(Self, Option<Tap>), opus::Error> {
        let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)?;
        encoder.set_bitrate(Bitrate::Bits(BITRATE))?;
        encoder.set_signal(Signal::Voice)?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(EXPECTED_LOSS_PERCENT)?;
        let (conditioner, tap) = match conditioning {
            Conditioning::Engine => {
                let (conditioner, tap) = Conditioner::new();
                (Some(conditioner), Some(tap))
            }
            Conditioning::Platform => (None, None),
        };
        let capture = Self {
            denoiser: DenoiseState::new(),
            conditioner,
            encoder,
            gate: Gate::default(),
            noise_suppression: true,
            next_index: 0,
            preroll: [None; PREROLL],
            scaled: [0.0; FRAME_SAMPLES],
            denoised: [0.0; FRAME_SAMPLES],
        };
        Ok((capture, tap))
    }

    /// Whether the frames sent are RNNoise's output (the default) or the input as captured, for
    /// a microphone whose noise is suppressed already, by the system or the device.
    pub fn set_noise_suppression(&mut self, on: bool) {
        self.noise_suppression = on;
    }

    /// Whether the speaker's echo is cancelled (the default), where the engine conditions the
    /// signal.
    pub fn set_echo_cancellation(&mut self, on: bool) {
        if let Some(conditioner) = &mut self.conditioner {
            conditioner.set_echo_cancellation(on);
        }
    }

    /// Counts `samples` the platform's capture callback hands over, the first of them captured
    /// at `at`, which aligns the speaker's echo with them where the engine cancels it.
    pub fn captured(&mut self, samples: usize, at: Instant) {
        if let Some(conditioner) = &mut self.conditioner {
            conditioner.captured(samples, at);
        }
    }

    /// Takes a captured frame and hands `send` what the gate lets through: nothing, the frame,
    /// or, when speech starts, the pre-roll and then the frame.
    pub fn process(&mut self, input: &[f32; FRAME_SAMPLES], mut send: impl FnMut(&Encoded)) -> Result<(), opus::Error> {
        let index = self.take_index();
        // A NaN, infinite or huge sample from a faulty driver would turn RNNoise's state to NaN for
        // good, and NaN in its spectrum is undefined behavior in its FFT: clip to full scale, as an
        // integer device would, and take NaN for silence.
        self.scaled.iter_mut().zip(input).for_each(|(scaled, sample)| {
            *scaled = if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) };
        });
        if let Some(conditioner) = &mut self.conditioner {
            conditioner.filter(&mut self.scaled);
        }
        self.scaled.iter_mut().for_each(|sample| *sample *= PCM_SCALE);
        let voice_probability = self.denoiser.process_frame(&mut self.denoised, &self.scaled);
        // Without suppression RNNoise still tells the gate what is speech; the frame goes out as captured.
        let signal = if self.noise_suppression { &mut self.denoised } else { &mut self.scaled };
        // The gate measures the level before the gain, which follows the voice's.
        let gate_level = Level::of(&signal.map(|sample| sample / PCM_SCALE));
        let level = if let Some(conditioner) = &mut self.conditioner {
            conditioner.gain(signal, voice_probability);
            // The gain may raise peaks beyond full scale.
            signal.iter_mut().for_each(|sample| *sample = limit(*sample / PCM_SCALE));
            Level::of(signal)
        } else {
            signal.iter_mut().for_each(|sample| *sample /= PCM_SCALE);
            gate_level
        };

        let decision = self.gate.decide(voice_probability, gate_level);
        let mut frame = Encoded { index, last: decision == Decision::Closes, level, len: 0, opus: [0; MAX_OPUS_LEN] };
        frame.len = self.encoder.encode_float(signal, &mut frame.opus)? as u16;

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

    /// Ends what input that stopped left unfinished, so capture can resume later: the pre-roll
    /// is stale by then, the speaker's echo is aligned anew with what is captured then, and a
    /// talkspurt the gate holds open gets a silent last frame, which `send` is handed.
    pub fn end(&mut self, send: impl FnOnce(&Encoded)) -> Result<(), opus::Error> {
        self.preroll = [None; PREROLL];
        if let Some(conditioner) = &mut self.conditioner {
            conditioner.stopped();
        }
        if !self.gate.end() {
            return Ok(());
        }
        let index = self.take_index();
        let mut frame = Encoded { index, last: true, level: Level::SILENT, len: 0, opus: [0; MAX_OPUS_LEN] };
        frame.len = self.encoder.encode_float(&[0.0; FRAME_SAMPLES], &mut frame.opus)? as u16;
        send(&frame);
        Ok(())
    }

    /// Counts frames the input didn't capture, while the platform's audio stopped, so the index
    /// stays the sender's clock: listeners measure delay against it.
    pub fn skip(&mut self, frames: u32) {
        self.next_index = self.next_index.wrapping_add(frames);
    }

    /// The next frame's index. It counts a frame the encoder fails on too, so the timeline
    /// stays true.
    fn take_index(&mut self) -> u32 {
        let index = self.next_index;
        self.next_index = self.next_index.wrapping_add(1);
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate;
    use crate::signal::{noise, voice};

    const SECOND: usize = SAMPLE_RATE as usize;

    /// Captures a signal with noise suppression, conditioned by the platform, and returns the
    /// frames sent.
    fn capture(signal: &[f32]) -> Vec<Encoded> {
        capture_with(signal, true, Conditioning::Platform)
    }

    fn capture_with(signal: &[f32], noise_suppression: bool, conditioning: Conditioning) -> Vec<Encoded> {
        let (mut capture, _) = Capture::new(conditioning).unwrap();
        capture.set_noise_suppression(noise_suppression);
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

    #[test]
    fn without_suppression_noise_still_sends_nothing_and_speech_goes_out_as_captured() {
        assert!(capture_with(&noise(3 * SECOND, 0.05), false, Conditioning::Platform).is_empty());

        let signal = voice(2 * SECOND, 0.3);
        let frames = signal.as_chunks::<FRAME_SAMPLES>().0;
        let sent = capture_with(&signal, false, Conditioning::Platform);
        assert!(sent.len() >= 150, "only {} frames of 2 s of speech sent", sent.len());
        for frame in sent {
            assert_eq!(frame.level, Level::of(&frames[frame.index as usize]), "frame {}", frame.index);
        }
    }

    #[test]
    fn conditioned_quiet_speech_goes_out_louder_and_noise_still_sends_nothing() {
        let loudest = |conditioning| {
            let sent = capture_with(&voice(5 * SECOND, 0.03), true, conditioning);
            assert!(sent.len() >= 400, "only {} frames of 5 s of speech sent", sent.len());
            sent.iter().map(|frame| frame.level.minus_dbov()).min().unwrap()
        };
        let (captured, conditioned) = (loudest(Conditioning::Platform), loudest(Conditioning::Engine));
        assert!(conditioned + 15 <= captured, "loudest frame at −{captured} dBov, conditioned at −{conditioned} dBov");

        let signal: Vec<f32> = [vec![0.0; SECOND], noise(3 * SECOND, 0.05), vec![0.0; SECOND]].concat();
        assert!(capture_with(&signal, true, Conditioning::Engine).is_empty());
    }

    #[test]
    fn conditioned_speech_is_captured_without_allocating() {
        let (mut capture, _tap) = Capture::new(Conditioning::Engine).unwrap();
        let signal = voice(2 * SECOND, 0.3);
        let (first, rest) = signal.as_chunks().0.split_first().unwrap();
        // RNNoise sets up its FFT on the first frame.
        capture.process(first, |_| ()).unwrap();
        let mut sent = 0;
        assert_no_alloc::assert_no_alloc(|| {
            for frame in rest {
                capture.process(frame, |_| sent += 1).unwrap();
            }
        });
        assert!(sent >= 150, "only {sent} frames of 2 s of speech sent");
    }

    #[test]
    fn ending_in_speech_sends_a_silent_last_frame_and_capture_starts_anew() {
        let (mut capture, _) = Capture::new(Conditioning::Platform).unwrap();
        let mut sent = Vec::new();
        for frame in voice(SECOND, 0.3).as_chunks().0 {
            capture.process(frame, |encoded| sent.push(*encoded)).unwrap();
        }
        assert!(!sent.last().unwrap().last, "still speaking");
        capture.end(|encoded| sent.push(*encoded)).unwrap();
        let ended = sent.last().unwrap();
        assert!(ended.last);
        assert_eq!(ended.level, Level::SILENT);
        assert_eq!(ended.index, 100);

        let mut nothing = true;
        capture.end(|_| nothing = false).unwrap();
        assert!(nothing, "a closed gate has nothing to end");
        let mut resumed = Vec::new();
        for frame in voice(SECOND, 0.3).as_chunks().0 {
            capture.process(frame, |encoded| resumed.push(*encoded)).unwrap();
        }
        // The input settles anew before the gate opens again, after the frame ended on.
        let first = resumed[0].index;
        assert!(first >= 101 + gate::SETTLE - PREROLL as u32, "resumed at frame {first}");
    }
}
