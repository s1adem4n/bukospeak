//! Evens out the microphone's signal where the platform's audio doesn't: a high-pass filter takes
//! out rumble and hum below the voice before RNNoise, and WebRTC's AGC2 brings speech to a
//! steady level after it, without raising the noise between words above −50 dBFS.
//!
//! Both are `sonora`'s, the Rust port of WebRTC's audio processing. Its `AudioProcessing`
//! allocates on every frame, so this puts the AGC's parts together itself; they take samples
//! scaled like 16-bit PCM, as RNNoise does. The speech probability is RNNoise's, which stands in
//! for AGC2's own voice detector, a smaller network of the same kind. Its limiter allocates
//! too, so the capture limits with the soft limiter the mix uses.

use bukospeak_proto::audio::{FRAME_SAMPLES, SAMPLE_RATE};
use sonora::high_pass_filter::HighPassFilter;
use sonora_agc2::adaptive_digital_gain_controller::{AdaptiveDigitalGainController, FrameInfo};
use sonora_agc2::common::{ADJACENT_SPEECH_FRAMES_THRESHOLD, SATURATION_PROTECTOR_INITIAL_HEADROOM_DB, float_s16_to_dbfs};
use sonora_agc2::fixed_digital_level_estimator::FixedDigitalLevelEstimator;
use sonora_agc2::noise_level_estimator::NoiseLevelEstimator;
use sonora_agc2::saturation_protector::SaturationProtector;
use sonora_agc2::speech_level_estimator::{AdaptiveDigitalConfig, SpeechLevelEstimator};

/// Who conditions the microphone's signal: takes out the rumble below the voice and evens out
/// its level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conditioning {
    /// The platform's audio, as iOS's voice processing unit does.
    Platform,
    /// The engine, for audio that passes the microphone's signal as it is.
    Engine,
}

pub struct Conditioner {
    high_pass: HighPassFilter,
    /// The frame the high-pass filter works on, in the form it takes.
    filtered: [Vec<f32>; 1],
    noise: NoiseLevelEstimator,
    speech: SpeechLevelEstimator,
    saturation: SaturationProtector,
    gain: AdaptiveDigitalGainController,
    /// The envelope of the frames after the gain, which AGC2 tracks in its limiter: the gain
    /// rises only so far that it stays below full scale while the speech level is uncertain.
    envelope: FixedDigitalLevelEstimator,
}

impl Conditioner {
    pub fn new() -> Self {
        let config = AdaptiveDigitalConfig::default();
        Self {
            high_pass: HighPassFilter::new(SAMPLE_RATE as i32, 1),
            filtered: [vec![0.0; FRAME_SAMPLES]],
            noise: NoiseLevelEstimator::default(),
            speech: SpeechLevelEstimator::new(&config, ADJACENT_SPEECH_FRAMES_THRESHOLD),
            saturation: SaturationProtector::new(SATURATION_PROTECTOR_INITIAL_HEADROOM_DB, ADJACENT_SPEECH_FRAMES_THRESHOLD),
            gain: AdaptiveDigitalGainController::new(config, ADJACENT_SPEECH_FRAMES_THRESHOLD),
            envelope: FixedDigitalLevelEstimator::new(FRAME_SAMPLES),
        }
    }

    /// Filters a frame, as captured, of samples scaled like 16-bit PCM.
    pub fn filter(&mut self, frame: &mut [f32; FRAME_SAMPLES]) {
        self.filtered[0].copy_from_slice(frame);
        self.high_pass.process_channels(&mut self.filtered);
        frame.copy_from_slice(&self.filtered[0]);
    }

    /// Applies the gain to a filtered and denoised frame, of samples scaled like 16-bit PCM, by
    /// the probability that it is speech. Its peaks may exceed full scale, for a limiter to bend.
    pub fn gain(&mut self, frame: &mut [f32; FRAME_SAMPLES], speech_probability: f32) {
        let (peak, energy) =
            frame.iter().fold((0.0f32, 0.0f32), |(peak, energy), sample| (peak.max(sample.abs()), energy + sample * sample));
        let rms_dbfs = float_s16_to_dbfs((energy / FRAME_SAMPLES as f32).sqrt());
        let noise_rms_dbfs = self.noise.analyze(&[frame]);
        self.speech.update(rms_dbfs, speech_probability);
        let speech_level_dbfs = self.speech.level_dbfs();
        self.saturation.analyze(speech_probability, float_s16_to_dbfs(peak), speech_level_dbfs);
        let info = FrameInfo {
            speech_probability,
            speech_level_dbfs,
            speech_level_reliable: self.speech.is_confident(),
            noise_rms_dbfs,
            headroom_db: self.saturation.headroom_db(),
            limiter_envelope_dbfs: float_s16_to_dbfs(self.envelope.last_audio_level()),
        };
        self.gain.process(&info, &mut [frame]);
        // Its levels per subframe are for a limiter that follows the envelope; the next frame
        // needs only the envelope's state.
        let _ = self.envelope.compute_level(&[frame]);
    }
}

#[cfg(test)]
mod tests {
    use bukospeak_proto::audio::Level;

    use super::*;
    use crate::signal::voice;

    const SECOND: usize = SAMPLE_RATE as usize;

    /// RNNoise scales like 16-bit PCM.
    const PCM_SCALE: f32 = 32_768.0;

    /// Conditions a signal, speech throughout, and returns its frames' levels after the gain.
    fn levels(signal: &[f32], speech_probability: f32) -> Vec<Level> {
        let mut conditioner = Conditioner::new();
        signal
            .as_chunks()
            .0
            .iter()
            .map(|frame| {
                let mut frame = frame.map(|sample| sample * PCM_SCALE);
                conditioner.filter(&mut frame);
                conditioner.gain(&mut frame, speech_probability);
                Level::of(&frame.map(|sample| sample / PCM_SCALE))
            })
            .collect()
    }

    /// The level of a frame of speech once the gain settled: the last second's loudest.
    fn settled(levels: &[Level]) -> u8 {
        levels[levels.len() - 100..].iter().map(|level| level.minus_dbov()).min().unwrap()
    }

    #[test]
    fn quiet_speech_is_raised_and_loud_speech_kept() {
        for (peak, raised) in [(0.01, 20..=40), (0.5, -1..=1)] {
            let signal = voice(20 * SECOND, peak);
            let captured = settled(&signal.as_chunks::<FRAME_SAMPLES>().0.iter().map(|frame| Level::of(frame)).collect::<Vec<_>>());
            let conditioned = settled(&levels(&signal, 1.0));
            assert!(
                raised.contains(&(i16::from(captured) - i16::from(conditioned))),
                "from −{captured} to −{conditioned} dBov at peak {peak}"
            );
        }
    }

    #[test]
    fn what_isnt_speech_gets_no_more_gain_than_it_started_with() {
        let speech = settled(&levels(&voice(20 * SECOND, 0.01), 1.0));
        let unsure = settled(&levels(&voice(20 * SECOND, 0.01), 0.0));
        assert!(unsure > speech + 10, "speech at −{speech} dBov, the same unsure at −{unsure} dBov");
    }

    #[test]
    fn the_filter_takes_out_hum() {
        let hum: Vec<f32> = (0..SECOND).map(|n| 0.5 * (std::f32::consts::TAU * 50.0 * n as f32 / SAMPLE_RATE as f32).sin()).collect();
        let mut conditioner = Conditioner::new();
        let filtered: Vec<f32> = hum
            .as_chunks()
            .0
            .iter()
            .flat_map(|frame| {
                let mut frame = frame.map(|sample| sample * PCM_SCALE);
                conditioner.filter(&mut frame);
                frame.map(|sample| sample / PCM_SCALE)
            })
            .collect();
        // After the filter's first 100 ms, 50 Hz at −9 dBov falls by over 20 dB.
        let after = Level::of(&filtered[SECOND / 10..]).minus_dbov();
        assert!(after >= Level::of(&hum).minus_dbov() + 20, "hum at −{after} dBov after the filter");
    }
}
