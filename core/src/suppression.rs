use std::fmt;
use std::sync::Arc;

use bukospeak_proto::audio::FRAME_SAMPLES;

pub const MAX_DELAY: usize = 31;

pub trait Suppressor: Send {
    fn delay(&self) -> usize;

    fn process(&mut self, frame: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]);

    fn reset(&mut self);
}

pub trait MakeSuppressor: Send + Sync {
    fn make(&self) -> Option<Box<dyn Suppressor>>;
}

#[derive(Clone)]
pub enum Suppression {
    Off,
    RNNoise,
    #[cfg(feature = "deepfilter")]
    DeepFilter,
    Plugged(Arc<dyn MakeSuppressor>),
}

impl Suppression {
    pub fn built_in() -> Self {
        #[cfg(feature = "deepfilter")]
        return Self::DeepFilter;
        #[cfg(not(feature = "deepfilter"))]
        Self::RNNoise
    }
}

impl fmt::Debug for Suppression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "Off",
            Self::RNNoise => "RNNoise",
            #[cfg(feature = "deepfilter")]
            Self::DeepFilter => "DeepFilter",
            Self::Plugged(_) => "Plugged",
        })
    }
}

pub(crate) enum Stage {
    Off,
    RNNoise,
    Suppressor(Box<dyn Suppressor>),
}

impl Stage {
    pub fn new(suppression: &Suppression) -> Self {
        match suppression {
            Suppression::Off => Self::Off,
            Suppression::RNNoise => Self::RNNoise,
            #[cfg(feature = "deepfilter")]
            Suppression::DeepFilter => match crate::deep_filter::DeepFilter::new() {
                Ok(deep_filter) => Self::Suppressor(Box::new(deep_filter)),
                // The model is built in, so it fails only on a bug.
                Err(_) => Self::RNNoise,
            },
            Suppression::Plugged(make) => match make.make() {
                Some(suppressor) => Self::Suppressor(suppressor),
                None => Self::new(&Suppression::built_in()),
            },
        }
    }

    pub fn delay(&self) -> usize {
        match self {
            Self::Off | Self::RNNoise => 0,
            Self::Suppressor(suppressor) => suppressor.delay().min(MAX_DELAY),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;

    use super::*;

    pub struct Delayed {
        frames: VecDeque<[f32; FRAME_SAMPLES]>,
        delay: usize,
    }

    impl Delayed {
        pub fn new(delay: usize) -> Self {
            let mut frames = VecDeque::with_capacity(delay + 1);
            frames.resize(delay, [0.0; FRAME_SAMPLES]);
            Self { frames, delay }
        }
    }

    impl Suppressor for Delayed {
        fn delay(&self) -> usize {
            self.delay
        }

        fn process(&mut self, frame: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]) {
            self.frames.push_back(*frame);
            *out = self.frames.pop_front().expect("one frame in, one out");
        }

        fn reset(&mut self) {
            self.frames.iter_mut().for_each(|frame| *frame = [0.0; FRAME_SAMPLES]);
        }
    }

    struct Failing;

    impl MakeSuppressor for Failing {
        fn make(&self) -> Option<Box<dyn Suppressor>> {
            None
        }
    }

    #[test]
    fn a_plugged_suppressor_that_cant_be_made_gives_way_to_the_best_built_in() {
        let stage = Stage::new(&Suppression::Plugged(Arc::new(Failing)));
        #[cfg(feature = "deepfilter")]
        assert!(matches!(stage, Stage::Suppressor(_)));
        #[cfg(not(feature = "deepfilter"))]
        assert!(matches!(stage, Stage::RNNoise));
    }
}
