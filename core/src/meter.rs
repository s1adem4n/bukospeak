//! How loud the microphone is, for a meter to show: what the platform's capture callback
//! hears, before anything processes it.
//!
//! The callback measures each buffer's peak; the level jumps up to it at once and falls back
//! slowly, as a meter's needle does, so a reader looking now and then sees every peak. Measured
//! by samples, not by a clock, it falls the same with buffers of any length.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bukospeak_proto::audio::SAMPLE_RATE;

/// The quietest level shown, in dB below full scale: a quiet room's noise.
const FLOOR_DB: f32 = -60.0;

/// How fast the level falls back, in dB per second.
const RELEASE_DB: f32 = 20.0;

/// The level, shared between the callback that measures it and those that show it. Lock-free,
/// so a realtime callback may measure every buffer.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    /// The level from 0, at or below [`FLOOR_DB`], to 1, at full scale, as an `f32`'s bits.
    level: Arc<AtomicU32>,
}

impl Meter {
    /// Measures samples the microphone captured, in −1.0..=1.0.
    pub fn measure(&self, samples: &[f32]) {
        // `max` passes over NaN, which a faulty driver may hand over.
        let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let fallen = self.level() - RELEASE_DB / -FLOOR_DB * samples.len() as f32 / SAMPLE_RATE as f32;
        self.level.store(level(peak).max(fallen).max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// Silences the meter: the microphone stopped.
    pub fn reset(&self) {
        self.level.store(0, Ordering::Relaxed);
    }

    /// The level from 0, silence or the quietest shown, to 1, full scale.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

/// A peak's level on the meter's scale.
fn level(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use bukospeak_proto::audio::FRAME_SAMPLES;

    use super::*;
    use crate::signal::voice;

    fn close(level: f32, expected: f32) -> bool {
        (level - expected).abs() < 0.01
    }

    #[test]
    fn the_level_is_the_peak_on_a_scale_from_the_floor_to_full_scale() {
        let meter = Meter::default();
        assert_eq!(meter.level(), 0.0);
        meter.measure(&[0.0; FRAME_SAMPLES]);
        assert_eq!(meter.level(), 0.0);
        meter.measure(&[0.0, -1.0, 0.5]);
        assert_eq!(meter.level(), 1.0);

        // 30 dB below full scale is half way up.
        let meter = Meter::default();
        meter.measure(&voice(FRAME_SAMPLES, 10f32.powf(-30.0 / 20.0)));
        assert!(close(meter.level(), 0.5), "{}", meter.level());
    }

    #[test]
    fn the_level_jumps_up_and_falls_back_slowly_however_long_the_buffers() {
        let meter = Meter::default();
        meter.measure(&[1.0]);
        // A second of silence: 20 dB of the 60 shown.
        for _ in 0..100 {
            meter.measure(&[0.0; FRAME_SAMPLES]);
        }
        assert!(close(meter.level(), 2.0 / 3.0), "{}", meter.level());
        meter.measure(&[0.0; SAMPLE_RATE as usize]);
        assert!(close(meter.level(), 1.0 / 3.0), "{}", meter.level());
        meter.measure(&[1.0]);
        assert_eq!(meter.level(), 1.0);
    }

    #[test]
    fn broken_samples_and_a_stop_leave_the_level_in_range() {
        let meter = Meter::default();
        meter.measure(&[f32::NAN, 0.0]);
        assert_eq!(meter.level(), 0.0);
        meter.measure(&[f32::INFINITY]);
        assert_eq!(meter.level(), 1.0);
        meter.reset();
        assert_eq!(meter.level(), 0.0);
    }

    #[test]
    fn measuring_doesnt_allocate() {
        let meter = Meter::default();
        let signal = voice(FRAME_SAMPLES, 0.3);
        assert_no_alloc::assert_no_alloc(|| meter.measure(&signal));
    }
}
