//! DeepFilterNet3, which suppresses noise far better than RNNoise, at costs RNNoise doesn't
//! have: the voice comes out 30 ms later, the model takes a third of a second to build, and it
//! allocates as it runs, so the uplink thread runs it, not the capture callback.

use bukospeak_proto::audio::FRAME_SAMPLES;
use df::{DEEPFILTERNET3, DfParams, DfTract, RuntimeParams};

pub(crate) struct DeepFilter {
    model: DfTract,
}

impl DeepFilter {
    /// Builds the model and runs it once, so a call's first frame doesn't wait for what its
    /// first run sets up.
    pub fn new() -> Result<Self, df::Error> {
        let model = DfTract::new(DfParams::from_bytes(DEEPFILTERNET3)?, &RuntimeParams::default())?;
        assert_eq!(model.hop_size(), FRAME_SAMPLES, "the model takes a frame at a time");
        let mut deep_filter = Self { model };
        deep_filter.process(&[0.0; FRAME_SAMPLES], &mut [0.0; FRAME_SAMPLES])?;
        deep_filter.reset()?;
        Ok(deep_filter)
    }

    /// Enhances the next frame, in −1.0..=1.0, into `enhanced`, which holds the voice of the
    /// model's delay before: three frames.
    pub fn process(&mut self, frame: &[f32; FRAME_SAMPLES], enhanced: &mut [f32; FRAME_SAMPLES]) -> Result<(), df::Error> {
        self.model.process(frame, enhanced).map(|_| ())
    }

    /// Starts anew, as after the input stopped: nothing from before comes out after.
    pub fn reset(&mut self) -> Result<(), df::Error> {
        self.model.reset()
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
                deep_filter.process(if frame == 0 { &click } else { &[0.0; FRAME_SAMPLES] }, &mut enhanced).unwrap();
                enhanced.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()))
            })
            .collect();
        let loudest = peaks.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(loudest, 3, "peaks per frame {peaks:?}");
    }
}
