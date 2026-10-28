//! DeepFilterNet3, which suppresses noise far better than RNNoise, at costs RNNoise doesn't
//! have: the voice comes out 30 ms later, and each frame takes its networks about a third of a
//! millisecond, so the uplink thread runs it, not the capture callback.

use bukospeak_proto::audio::FRAME_SAMPLES;
use df::{DEEPFILTERNET3, DeepFilterNet, RuntimeParams};

pub(crate) struct DeepFilter {
    model: DeepFilterNet,
}

impl DeepFilter {
    /// Builds the model from its built-in weights.
    pub fn new() -> Result<Self, df::Error> {
        let model = DeepFilterNet::new(DEEPFILTERNET3, &RuntimeParams::default())?;
        assert_eq!(model.hop_size(), FRAME_SAMPLES, "the model takes a frame at a time");
        Ok(Self { model })
    }

    /// Enhances the next frame, in −1.0..=1.0, into `enhanced`, which holds the voice of the
    /// model's delay before: three frames.
    pub fn process(&mut self, frame: &[f32; FRAME_SAMPLES], enhanced: &mut [f32; FRAME_SAMPLES]) {
        // The local SNR it estimates matters only to the model, which chose by it what the frame
        // got of its stages.
        let _ = self.model.process(frame, enhanced);
    }

    /// Starts anew, as after the input stopped: nothing from before comes out after.
    pub fn reset(&mut self) {
        self.model.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_voice_comes_out_three_frames_later() {
        let mut deep_filter = DeepFilter::new().unwrap();
        assert_eq!(deep_filter.model.delay(), 3 * FRAME_SAMPLES);
        // A click in the first frame, after which the output is silent until it comes out.
        let mut click = [0.0; FRAME_SAMPLES];
        click[100] = 0.5;
        let mut enhanced = [0.0; FRAME_SAMPLES];
        let peaks: Vec<f32> = (0..5)
            .map(|frame| {
                deep_filter.process(if frame == 0 { &click } else { &[0.0; FRAME_SAMPLES] }, &mut enhanced);
                enhanced.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()))
            })
            .collect();
        let loudest = peaks.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(loudest, 3, "peaks per frame {peaks:?}");
    }

    #[test]
    fn frames_are_enhanced_without_allocating() {
        let mut deep_filter = DeepFilter::new().unwrap();
        let mut enhanced = [0.0; FRAME_SAMPLES];
        // Noise rising from silence.
        let mut state: u32 = 0x1234_5678;
        let frames: Vec<[f32; FRAME_SAMPLES]> = (0..100)
            .map(|frame| {
                std::array::from_fn(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    (state as f32 / u32::MAX as f32 - 0.5) * frame as f32 / 100.
                })
            })
            .collect();
        assert_no_alloc::assert_no_alloc(|| {
            for frame in &frames {
                deep_filter.process(frame, &mut enhanced);
            }
            deep_filter.reset();
        });
    }
}
