//! The captured voice, from filtered microphone frames to the Opus frames the uplink sends.
//!
//! The uplink thread owns it and hands it what the capture callback captured, in order: frames,
//! filtered where the engine conditions the signal, and where the input stopped. It suppresses
//! noise with DeepFilterNet3 where it has it, else with RNNoise, which tells the gate what is
//! speech either way; evens out the level where the platform doesn't, lets the gate decide, and
//! encodes every frame, sent or not, so the encoder stays warm. Frames that went missing on the
//! way, or came too late, end the speech they were in as if the input had stopped there, and so
//! does a change of how noise is suppressed: the signal before and after doesn't line up.

use std::time::{Duration, Instant};

use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES, Level, MAX_OPUS_LEN, SAMPLE_RATE};
use nnnoiseless::DenoiseState;
use opus::{Application, Bitrate, Channels, Encoder, Signal};

use crate::conditioner::{Conditioning, Gain};
#[cfg(feature = "deepfilter")]
use crate::deep_filter::DeepFilter;
use crate::gate::{Decision, Gate};
use crate::jitter;
use crate::limiter::limit;

/// Low enough that libopus stays in hybrid mode, where in-band FEC exists.
const BITRATE: i32 = 48_000;

/// The loss the encoder spends FEC on; it only adds FEC when told to expect loss.
const EXPECTED_LOSS_PERCENT: i32 = 10;

/// Frames sent ahead of the one that opens the gate: 50 ms, so word onsets survive.
const PREROLL: usize = 5;

/// A frame older than this when it is processed is discarded: no listener waits that long for one.
const MAX_AGE: Duration = FRAME_DURATION.saturating_mul(jitter::MAX_TARGET);

/// RNNoise takes and returns samples scaled like 16-bit PCM.
pub(crate) const PCM_SCALE: f32 = 32_768.0;

const _: () = assert!(DenoiseState::FRAME_SIZE == FRAME_SAMPLES);
// An encoded frame's length is stored as a u16.
const _: () = assert!(MAX_OPUS_LEN <= u16::MAX as usize);

/// What the capture callback hands the uplink, in order.
#[derive(Clone, Copy, Debug)]
#[expect(clippy::large_enum_variant, reason = "a ring's slots hold it in place, so boxing would only allocate")]
pub(crate) enum Captured {
    /// A frame of samples in −1.0..=1.0, filtered where the engine conditions the signal, and
    /// when it was handed over.
    Frame { index: u32, queued: Instant, samples: [f32; FRAME_SAMPLES] },
    /// The input stopped; `index` is kept free for the last frame of the speech it cut short.
    Stopped { index: u32 },
}

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

/// How noise is suppressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suppression {
    /// It isn't: the frames go out as captured.
    Off,
    RNNoise,
    #[cfg(feature = "deepfilter")]
    DeepFilter,
}

pub struct Capture {
    /// RNNoise, which suppresses noise where DeepFilterNet3 doesn't, and tells the gate what is
    /// speech.
    denoiser: Box<DenoiseState<'static>>,
    #[cfg(feature = "deepfilter")]
    deep_filter: Option<DeepFilter>,
    #[cfg(feature = "deepfilter")]
    enhanced: [f32; FRAME_SAMPLES],
    /// None where the platform's audio evens out the level.
    gain: Option<Gain>,
    encoder: Encoder,
    gate: Gate,
    noise_suppression: bool,
    /// How the frames before were suppressed, if there were any.
    suppressed: Option<Suppression>,
    /// The index the next frame should have, unless the input stopped before it.
    expected: Option<u32>,
    /// The latest frames the gate kept back, oldest first.
    preroll: [Option<Encoded>; PREROLL],
    scaled: [f32; FRAME_SAMPLES],
    denoised: [f32; FRAME_SAMPLES],
}

impl Capture {
    pub fn new(conditioning: Conditioning) -> Result<Self, opus::Error> {
        let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)?;
        encoder.set_bitrate(Bitrate::Bits(BITRATE))?;
        encoder.set_signal(Signal::Voice)?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(EXPECTED_LOSS_PERCENT)?;
        Ok(Self {
            denoiser: DenoiseState::new(),
            #[cfg(feature = "deepfilter")]
            deep_filter: None,
            #[cfg(feature = "deepfilter")]
            enhanced: [0.0; FRAME_SAMPLES],
            gain: (conditioning == Conditioning::Engine).then(Gain::default),
            encoder,
            gate: Gate::default(),
            noise_suppression: true,
            suppressed: None,
            expected: None,
            preroll: [None; PREROLL],
            scaled: [0.0; FRAME_SAMPLES],
            denoised: [0.0; FRAME_SAMPLES],
        })
    }

    /// Whether noise is suppressed (the default), or the frames go out as captured, for a
    /// microphone whose noise is suppressed already, by the system or the device.
    pub fn set_noise_suppression(&mut self, on: bool) {
        self.noise_suppression = on;
    }

    /// The capture suppressing noise with DeepFilterNet3 rather than RNNoise.
    #[cfg(feature = "deepfilter")]
    pub(crate) fn with_deep_filter(mut self, deep_filter: DeepFilter) -> Self {
        self.deep_filter = Some(deep_filter);
        self
    }

    /// How noise is suppressed now.
    pub fn suppression(&self) -> Suppression {
        #[cfg(feature = "deepfilter")]
        if self.noise_suppression && self.deep_filter.is_some() {
            return Suppression::DeepFilter;
        }
        if self.noise_suppression { Suppression::RNNoise } else { Suppression::Off }
    }

    /// Takes what the capture callback handed over, processed at `now`, and hands `send` what the
    /// gate lets through: nothing, the frame, or, when speech starts, the pre-roll and then the
    /// frame; and where speech ends early, its last frame.
    pub(crate) fn take(&mut self, captured: &Captured, now: Instant, mut send: impl FnMut(&Encoded)) -> Result<(), opus::Error> {
        match *captured {
            Captured::Frame { index, queued, ref samples } => {
                // Too late to be heard, the frame counts as lost.
                if now.saturating_duration_since(queued) > MAX_AGE {
                    return Ok(());
                }
                if let Some(expected) = self.expected
                    && expected != index
                {
                    // The first frame missing is free for the last.
                    self.end(expected, &mut send)?;
                }
                self.expected = Some(index.wrapping_add(1));
                let suppression = self.suppression();
                match self.suppressed.replace(suppression) {
                    Some(before) if before != suppression => {
                        // The frame is lost to the last frame of the speech before, and the gate
                        // learns the room anew, as it sounds now.
                        self.end(index, send)?;
                        self.gate = Gate::default();
                        Ok(())
                    }
                    _ => self.process(index, samples, suppression, send),
                }
            }
            Captured::Stopped { index } => {
                // Where frames went missing before the stop, the first of them is free for the last.
                let index = self.expected.take().unwrap_or(index);
                self.end(index, send)
            }
        }
    }

    fn process(
        &mut self,
        index: u32,
        samples: &[f32; FRAME_SAMPLES],
        suppression: Suppression,
        mut send: impl FnMut(&Encoded),
    ) -> Result<(), opus::Error> {
        // Told from the frame as captured: suppression silences noise too.
        let captured = Level::of(samples) != Level::SILENT;
        #[cfg(feature = "deepfilter")]
        let samples = match &mut self.deep_filter {
            Some(deep_filter) if suppression == Suppression::DeepFilter => {
                if deep_filter.process(samples, &mut self.enhanced).is_ok() {
                    &self.enhanced
                } else {
                    // The model fails only as it never should, on a bug: the frame goes out as
                    // captured, and RNNoise takes over from the next.
                    self.deep_filter = None;
                    samples
                }
            }
            _ => samples,
        };
        self.scaled.iter_mut().zip(samples).for_each(|(scaled, sample)| *scaled = sample * PCM_SCALE);
        let voice_probability = self.denoiser.process_frame(&mut self.denoised, &self.scaled);
        // Unless it suppresses the noise, RNNoise just tells the gate what is speech, in the frame
        // as it goes out.
        let signal = if suppression == Suppression::RNNoise { &mut self.denoised } else { &mut self.scaled };
        // The gate measures the level before the gain, which follows the voice's.
        let gate_level = Level::of(&signal.map(|sample| sample / PCM_SCALE));
        let level = if let Some(gain) = &mut self.gain {
            gain.apply(signal, voice_probability);
            // The gain may raise peaks beyond full scale.
            signal.iter_mut().for_each(|sample| *sample = limit(*sample / PCM_SCALE));
            Level::of(signal)
        } else {
            signal.iter_mut().for_each(|sample| *sample /= PCM_SCALE);
            gate_level
        };

        let decision = self.gate.decide(voice_probability, gate_level, captured);
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

    /// Ends what the input left unfinished where it stopped, or frames went missing, at the
    /// free `index`: the pre-roll is stale by then, and speech the gate holds open gets a silent
    /// last frame there.
    fn end(&mut self, index: u32, send: impl FnOnce(&Encoded)) -> Result<(), opus::Error> {
        self.preroll = [None; PREROLL];
        #[cfg(feature = "deepfilter")]
        if let Some(deep_filter) = &mut self.deep_filter
            && deep_filter.reset().is_err()
        {
            // As in processing, the model fails only as it never should; RNNoise takes over.
            self.deep_filter = None;
        }
        if !self.gate.end() {
            return Ok(());
        }
        let mut frame = Encoded { index, last: true, level: Level::SILENT, len: 0, opus: [0; MAX_OPUS_LEN] };
        frame.len = self.encoder.encode_float(&[0.0; FRAME_SAMPLES], &mut frame.opus)? as u16;
        send(&frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate;
    use crate::signal::{noise, voice};

    const SECOND: usize = SAMPLE_RATE as usize;

    /// Frames of a signal as the capture callback hands them over, numbered from `first` and
    /// queued at `start`.
    fn frames(signal: &[f32], first: u32, start: Instant) -> Vec<Captured> {
        let indices = std::iter::successors(Some(first), |index: &u32| Some(index.wrapping_add(1)));
        signal
            .as_chunks()
            .0
            .iter()
            .zip(indices)
            .map(|(samples, index)| Captured::Frame { index, queued: start, samples: *samples })
            .collect()
    }

    /// Captures a signal with noise suppression, conditioned by the platform, and returns the
    /// frames sent.
    fn capture(signal: &[f32]) -> Vec<Encoded> {
        capture_with(signal, true, Conditioning::Platform)
    }

    fn capture_with(signal: &[f32], noise_suppression: bool, conditioning: Conditioning) -> Vec<Encoded> {
        let mut capture = Capture::new(conditioning).unwrap();
        capture.set_noise_suppression(noise_suppression);
        let start = Instant::now();
        let mut sent = Vec::new();
        for captured in frames(signal, 0, start) {
            capture.take(&captured, start, |encoded| sent.push(*encoded)).unwrap();
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
        // The voice starts at frame 100, after digital silence, in a room not heard yet: the
        // gate opens once the denoiser settled, with the pre-roll before.
        let opens = 100 + gate::SETTLE;
        assert!((opens - PREROLL as u32..opens + 5).contains(&first), "starts at frame {first}");
        assert!(sent.len() >= 200, "only {} frames of 2 s of speech sent", sent.len());
        assert_eq!(sent.iter().filter(|frame| frame.last).count(), 1);
        assert!(sent.last().unwrap().last);
    }

    #[test]
    fn speech_on_unmuting_in_a_room_heard_before_is_sent_from_its_onset() {
        // Muted, the microphone hears digital silence.
        let signal: Vec<f32> = [noise(SECOND, 0.003), vec![0.0; SECOND], voice(SECOND, 0.3)].concat();
        let first = capture(&signal)[0].index;
        assert!(first <= 200, "starts at frame {first}");
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
        let mut capture = Capture::new(Conditioning::Engine).unwrap();
        let start = Instant::now();
        let frames = frames(&voice(2 * SECOND, 0.3), 0, start);
        let (first, rest) = frames.split_first().unwrap();
        // RNNoise sets up its FFT on the first frame.
        capture.take(first, start, |_| ()).unwrap();
        let mut sent = 0;
        assert_no_alloc::assert_no_alloc(|| {
            for captured in rest {
                capture.take(captured, start, |_| sent += 1).unwrap();
            }
        });
        assert!(sent >= 150, "only {sent} frames of 2 s of speech sent");
    }

    /// Takes everything captured at `start`, and returns the frames sent.
    fn take_all(capture: &mut Capture, captured: impl IntoIterator<Item = Captured>, now: Instant) -> Vec<Encoded> {
        let mut sent = Vec::new();
        for captured in captured {
            capture.take(&captured, now, |encoded| sent.push(*encoded)).unwrap();
        }
        sent
    }

    #[test]
    fn stopping_in_speech_sends_a_silent_last_frame_at_the_index_kept_free_and_capture_starts_anew() {
        let mut capture = Capture::new(Conditioning::Platform).unwrap();
        let start = Instant::now();
        let sent = take_all(&mut capture, frames(&voice(SECOND, 0.3), 0, start), start);
        assert!(!sent.last().unwrap().last, "still speaking");
        let ended = take_all(&mut capture, [Captured::Stopped { index: 100 }], start);
        assert_eq!(ended.len(), 1);
        assert!(ended[0].last);
        assert_eq!((ended[0].index, ended[0].level), (100, Level::SILENT));

        assert!(take_all(&mut capture, [Captured::Stopped { index: 101 }], start).is_empty(), "a closed gate has nothing to end");
        // Resumed, the next frames carry on from wherever the input's clock went meanwhile.
        let resumed = take_all(&mut capture, frames(&voice(SECOND, 0.3), 150, start), start);
        // The gate heard the room in the voice's pauses, so it opens again at once, with a
        // pre-roll from after the stop; the talkspurt runs on without a missing frame.
        assert_eq!(resumed[0].index, 150);
        assert!(resumed.windows(2).all(|pair| pair[1].index == pair[0].index + 1));
    }

    #[test]
    fn frames_missing_in_speech_end_it_at_the_first_of_them() {
        let mut capture = Capture::new(Conditioning::Platform).unwrap();
        let start = Instant::now();
        let captured = frames(&voice(2 * SECOND, 0.3), 0, start)
            .into_iter()
            .filter(|captured| !matches!(captured, Captured::Frame { index: 100..110, .. }));
        let sent = take_all(&mut capture, captured, start);
        let before: Vec<&Encoded> = sent.iter().filter(|frame| frame.index < 110).collect();
        assert_eq!(before.last().unwrap().index, 100);
        assert!(before.last().unwrap().last, "the speech before the gap ends at its first missing frame");
        assert_eq!(before.iter().filter(|frame| frame.last).count(), 1);
        assert!(sent.iter().any(|frame| frame.index >= 110), "the speech after the gap is sent");
    }

    #[test]
    fn a_stop_after_missing_frames_ends_the_speech_at_the_first_of_them() {
        let mut capture = Capture::new(Conditioning::Platform).unwrap();
        let start = Instant::now();
        assert!(!take_all(&mut capture, frames(&voice(SECOND, 0.3), 0, start), start).is_empty());
        let ended = take_all(&mut capture, [Captured::Stopped { index: 103 }], start);
        assert_eq!(ended.iter().map(|frame| (frame.index, frame.last)).collect::<Vec<_>>(), [(100, true)]);
    }

    #[test]
    fn frames_too_late_to_be_heard_are_discarded_as_missing() {
        let mut capture = Capture::new(Conditioning::Platform).unwrap();
        let start = Instant::now();
        let sent = take_all(&mut capture, frames(&voice(SECOND, 0.3), 0, start), start);
        assert!(!sent.is_empty());
        let late = frames(&voice(3 * FRAME_SAMPLES, 0.3), 100, start);
        let sent = take_all(&mut capture, late, start + MAX_AGE + FRAME_DURATION);
        assert!(sent.is_empty(), "late frames sent: {:?}", sent.iter().map(|frame| frame.index).collect::<Vec<_>>());
    }

    #[test]
    fn indices_wrap_around_without_a_gap() {
        let mut capture = Capture::new(Conditioning::Platform).unwrap();
        let start = Instant::now();
        let sent = take_all(&mut capture, frames(&voice(2 * SECOND, 0.3), u32::MAX - 100, start), start);
        assert_eq!(sent.iter().filter(|frame| frame.last).count(), 0, "no end where the index wrapped");
        assert!(sent.iter().any(|frame| frame.index < 100));
    }

    #[cfg(feature = "deepfilter")]
    fn deep_filtered() -> Capture {
        Capture::new(Conditioning::Platform).unwrap().with_deep_filter(DeepFilter::new().unwrap())
    }

    #[test]
    #[cfg(feature = "deepfilter")]
    fn deep_filtered_speech_is_sent_and_noise_sends_nothing() {
        let start = Instant::now();
        let signal: Vec<f32> = [vec![0.0; SECOND], noise(3 * SECOND, 0.05), vec![0.0; SECOND]].concat();
        let mut capture = deep_filtered();
        assert_eq!(capture.suppression(), Suppression::DeepFilter);
        assert!(take_all(&mut capture, frames(&signal, 0, start), start).is_empty());

        let sent = take_all(&mut deep_filtered(), frames(&voice(2 * SECOND, 0.3), 0, start), start);
        assert!(sent.len() >= 150, "only {} frames of 2 s of speech sent", sent.len());
    }

    #[test]
    #[cfg(feature = "deepfilter")]
    fn deep_filtered_speech_after_noise_is_sent_from_its_onset() {
        let start = Instant::now();
        // DeepFilterNet3 silences much of the noise to digital zeros, which the gate mustn't
        // take for input that stopped: it learns the room meanwhile.
        let mut signal = noise(3 * SECOND, 0.02);
        signal[2 * SECOND..].iter_mut().zip(voice(SECOND, 0.3)).for_each(|(sample, voice)| *sample += voice);
        let sent = take_all(&mut deep_filtered(), frames(&signal, 0, start), start);
        // The voice starts at frame 200, and comes out of the model 3 frames later.
        let first = sent.first().expect("the speech is sent").index;
        assert!(first <= 203, "starts at frame {first}");
    }

    #[test]
    #[cfg(feature = "deepfilter")]
    fn changing_the_suppression_ends_the_speech_at_the_frame_it_changes_on() {
        let mut capture = deep_filtered();
        let start = Instant::now();
        let captured = frames(&voice(3 * SECOND, 0.3), 0, start);
        let sent = take_all(&mut capture, captured[..100].iter().copied(), start);
        assert!(!sent.last().unwrap().last, "still speaking");
        capture.set_noise_suppression(false);
        let sent = take_all(&mut capture, captured[100..].iter().copied(), start);
        assert!(sent[0].last && sent[0].index == 100, "the speech ends at frame 100");
        // The gate learns the room anew, so the speech goes on once the denoiser settled.
        assert!(sent[1].index > 100 && !sent.last().unwrap().last, "the speech goes on");
    }

    #[test]
    #[cfg(feature = "deepfilter")]
    fn after_a_stop_deep_filter_starts_anew() {
        let mut capture = deep_filtered();
        let start = Instant::now();
        let signal = voice(SECOND, 0.3);
        take_all(&mut capture, frames(&signal, 0, start), start);
        take_all(&mut capture, [Captured::Stopped { index: 100 }], start);

        let frame = signal.as_chunks::<FRAME_SAMPLES>().0[0];
        let mut new = DeepFilter::new().unwrap();
        let (mut after_stop, mut fresh) = ([0.0; FRAME_SAMPLES], [0.0; FRAME_SAMPLES]);
        // Through the model's delay, so what comes out is what went in after the stop.
        for _ in 0..4 {
            capture.deep_filter.as_mut().unwrap().process(&frame, &mut after_stop).unwrap();
            new.process(&frame, &mut fresh).unwrap();
        }
        assert_eq!(after_stop, fresh);
    }
}
