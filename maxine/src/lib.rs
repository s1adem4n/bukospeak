//! NVIDIA Maxine's audio effects, which NVIDIA's runtime runs on the GPUs of RTX cards. The
//! runtime and the effects are loaded while running, from a download laid out as NVIDIA's SDK
//! lays them out:
//!
//! ```text
//! bin/NVAudioEffects.dll
//! bin/external/{cuda,nvtrt,openssl}/bin/*.dll
//! features/nvafx<effect>/bin/nvafx<effect>.dll
//! features/nvafx<effect>/models/<architecture>/*.trtpkg
//! license/*.pdf
//! ```
//!
//! Only Windows has them; elsewhere [`Runtime::load`] fails and [`detect`] finds no card.

mod detect;
mod runtime;

use std::str::FromStr;

pub use detect::detect;
pub use runtime::{Effect, Runtime, Severity, Status};

/// The sample rate the effects run at.
pub const SAMPLE_RATE: u32 = 48_000;

/// The samples of a frame, in and out: 10 ms.
pub const FRAME: usize = 480;

/// NVIDIA's runtime in a download, each file after those it uses. The SDK's NVRTC is left out:
/// nothing loads it.
pub const RUNTIME: [&str; 6] = [
    "bin/external/cuda/bin/cublasLt64_12.dll",
    "bin/external/cuda/bin/cublas64_12.dll",
    "bin/external/cuda/bin/cufft64_11.dll",
    "bin/external/nvtrt/bin/nvinfer_10.dll",
    "bin/external/openssl/bin/libcrypto-3-x64.dll",
    "bin/NVAudioEffects.dll",
];

/// The folder of NVIDIA's licences in a download.
pub const LICENSES: &str = "license";

/// What the names of the downloads begin with, before an architecture's.
const DOWNLOAD: &str = "maxine-";

/// A generation of RTX cards, for each of which NVIDIA builds the models anew.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Architecture {
    Turing,
    Ampere,
    Ada,
    Blackwell,
}

impl Architecture {
    pub const ALL: [Self; 4] = [Self::Turing, Self::Ampere, Self::Ada, Self::Blackwell];

    /// Its name, as NVIDIA's folders of models go by it: `ampere`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Turing => "turing",
            Self::Ampere => "ampere",
            Self::Ada => "ada",
            Self::Blackwell => "blackwell",
        }
    }

    /// The name of the download a server offers for it: `maxine-ampere`.
    pub fn download(self) -> String {
        format!("{DOWNLOAD}{}", self.name())
    }

    /// The architecture whose download is called `name`.
    pub fn of_download(name: &str) -> Option<Self> {
        name.strip_prefix(DOWNLOAD)?.parse().ok()
    }

    /// The generation of a card of CUDA compute capability `major.minor`, if it is one the
    /// effects run on: of the GeForce and workstation cards, with Tensor Cores.
    pub fn of_compute_capability(major: i32, minor: i32) -> Option<Self> {
        match (major, minor) {
            (7, 5) => Some(Self::Turing),
            (8, 0 | 6) => Some(Self::Ampere),
            (8, 9) => Some(Self::Ada),
            (10 | 12, _) => Some(Self::Blackwell),
            _ => None,
        }
    }
}

impl FromStr for Architecture {
    type Err = Error;

    /// The architecture by its name, as `ampere`.
    fn from_str(name: &str) -> Result<Self, Error> {
        Self::ALL.into_iter().find(|architecture| architecture.name() == name).ok_or_else(|| Error::Architecture(name.to_owned()))
    }
}

/// An effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Room echo and noise removed.
    DereverbDenoiser,
    /// The voice as if from a studio's microphone, with room echo and noise removed.
    StudioVoiceLowLatency,
}

impl Kind {
    pub const ALL: [Self; 2] = [Self::DereverbDenoiser, Self::StudioVoiceLowLatency];

    /// Its name, as NVIDIA's SDK selects it and NGC lists it: `dereverb_denoiser`.
    pub fn name(self) -> &'static str {
        match self {
            Self::DereverbDenoiser => "dereverb_denoiser",
            Self::StudioVoiceLowLatency => "studio_voice_low_latency",
        }
    }

    /// Its folder under `features/` in a download.
    pub fn feature(self) -> &'static str {
        match self {
            Self::DereverbDenoiser => "nvafxdereverbdenoiser",
            Self::StudioVoiceLowLatency => "nvafxstudiovoicelowlatency",
        }
    }

    /// How many frames later the voice comes out than it went in: the runtime takes a frame at
    /// a time, but runs the model on eight, as measured on an RTX 3060.
    pub fn delay(self) -> usize {
        match self {
            Self::DereverbDenoiser => 8,
            Self::StudioVoiceLowLatency => 7,
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::DereverbDenoiser => "dereverb_denoiser_48k.trtpkg",
            Self::StudioVoiceLowLatency => "studio_voice_low_latency_48k.trtpkg",
        }
    }
}

impl FromStr for Kind {
    type Err = Error;

    /// The effect by its name, as `dereverb_denoiser`.
    fn from_str(name: &str) -> Result<Self, Error> {
        Self::ALL.into_iter().find(|kind| kind.name() == name).ok_or_else(|| Error::Kind(name.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("NVIDIA's runtime runs only on Windows")]
    Unsupported,
    #[error("no architecture is called {0}")]
    Architecture(String),
    #[error("no effect is called {0}")]
    Kind(String),
    #[error("loading {path}: {source}")]
    Library { path: std::path::PathBuf, source: libloading::Error },
    #[error("NVIDIA's runtime has no {name:?}: {source}")]
    Symbol { name: &'static std::ffi::CStr, source: libloading::Error },
    #[error("{call}: {status}")]
    Call { call: &'static str, status: Status },
    #[error("CUDA's {call}: error {code}")]
    Cuda { call: &'static str, code: i32 },
    #[error("the effect takes frames of {supported:?} samples, not {FRAME}")]
    Frame { supported: Vec<u32> },
    #[error("{0} can't be passed to the runtime: it isn't UTF-8 or holds a nul")]
    Path(std::path::PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures_and_effects_go_by_their_names() {
        for architecture in Architecture::ALL {
            assert_eq!(architecture.name().parse::<Architecture>().unwrap(), architecture);
        }
        for kind in Kind::ALL {
            assert_eq!(kind.name().parse::<Kind>().unwrap(), kind);
        }
        assert!("kepler".parse::<Architecture>().is_err());
    }

    #[test]
    fn each_architecture_has_a_download_of_its_own() {
        for architecture in Architecture::ALL {
            assert_eq!(Architecture::of_download(&architecture.download()), Some(architecture));
        }
        assert_eq!(Architecture::of_download("ampere"), None);
        assert_eq!(Architecture::of_download("maxine-../ampere"), None);
    }

    #[test]
    fn rtx_cards_have_architectures_and_others_none() {
        assert_eq!(Architecture::of_compute_capability(7, 5), Some(Architecture::Turing));
        assert_eq!(Architecture::of_compute_capability(8, 6), Some(Architecture::Ampere));
        assert_eq!(Architecture::of_compute_capability(8, 9), Some(Architecture::Ada));
        assert_eq!(Architecture::of_compute_capability(12, 0), Some(Architecture::Blackwell));
        // Pascal, and Jetson's Ampere and Hopper, which aren't in PCs.
        for (major, minor) in [(6, 1), (8, 7), (9, 0)] {
            assert_eq!(Architecture::of_compute_capability(major, minor), None);
        }
    }

    #[test]
    fn the_runtime_comes_last_after_what_it_uses() {
        assert_eq!(RUNTIME.last(), Some(&"bin/NVAudioEffects.dll"));
    }
}
