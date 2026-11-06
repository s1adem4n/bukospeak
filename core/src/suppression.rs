//! How the microphone's noise is suppressed: not at all, by RNNoise or DeepFilterNet3, which
//! the engine has built in, or by a suppressor the app plugs in, as one that runs on the GPU.

use std::fmt;
use std::sync::Arc;

use bukospeak_proto::audio::FRAME_SAMPLES;

/// The longest delay a suppressor may have, in frames.
pub const MAX_DELAY: usize = 31;

/// Takes the noise out of the voice, frame by frame. RNNoise still tells the gate what is
/// speech, in the frames as they come out.
pub trait Suppressor: Send {
    /// How many frames later the voice comes out than it went in, up to [`MAX_DELAY`].
    fn delay(&self) -> usize;

    /// Suppresses the noise in the next frame, in −1.0..=1.0, into `out`, which holds the voice
    /// of [`delay`](Self::delay) frames before.
    fn process(&mut self, frame: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]);

    /// Starts anew, as after the input stopped: nothing from before comes out after.
    fn reset(&mut self);
}

/// Makes the suppressor an app plugged in: for each call, while it joins, and when it is chosen
/// during one, on a thread of its own, while the suppression before goes on.
pub trait MakeSuppressor: Send + Sync {
    /// The suppressor, or None if it can't be made now; it tells the app why, and the best
    /// suppression built in takes its place.
    fn make(&self) -> Option<Box<dyn Suppressor>>;
}

/// How the microphone's noise is suppressed.
#[derive(Clone)]
pub enum Suppression {
    /// It isn't: the frames go out as captured.
    Off,
    RNNoise,
    /// DeepFilterNet3, which suppresses noise far better than RNNoise, 30 ms later.
    #[cfg(feature = "deepfilter")]
    DeepFilter,
    Plugged(Arc<dyn MakeSuppressor>),
}

impl Suppression {
    /// The best suppression the engine has built in.
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

/// The suppression a capture runs, made from a [`Suppression`].
pub(crate) enum Stage {
    Off,
    RNNoise,
    Suppressor(Box<dyn Suppressor>),
}

impl Stage {
    /// Makes the stage, which takes a plugged-in suppressor what it takes to load.
    pub fn new(suppression: &Suppression) -> Self {
        match suppression {
            Suppression::Off => Self::Off,
            Suppression::RNNoise => Self::RNNoise,
            #[cfg(feature = "deepfilter")]
            Suppression::DeepFilter => match crate::deep_filter::DeepFilter::new() {
                Ok(deep_filter) => Self::Suppressor(Box::new(deep_filter)),
                // The model is built in, so it fails to build only on a bug; RNNoise suppresses
                // the noise then.
                Err(_) => Self::RNNoise,
            },
            Suppression::Plugged(make) => match make.make() {
                Some(suppressor) => Self::Suppressor(suppressor),
                // The maker told the app why.
                None => Self::new(&Suppression::built_in()),
            },
        }
    }

    /// How many frames later the voice comes out.
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

    /// A suppressor that passes the voice through, `delay` frames later.
    pub struct Delayed {
        frames: VecDeque<[f32; FRAME_SAMPLES]>,
        delay: usize,
    }

    impl Delayed {
        pub fn new(delay: usize) -> Self {
            // Room for the frame in beside those held, so processing doesn't allocate.
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
