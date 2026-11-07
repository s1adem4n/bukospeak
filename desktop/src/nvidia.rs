//! NVIDIA's effects on an RTX card, which only Windows runs: installed from the download the
//! server offers for the card's generation, and run as the call's noise suppression.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bukospeak_core::proto::audio::FRAME_SAMPLES;
use bukospeak_core::{MakeSuppressor, Suppression, Suppressor};
pub use bukospeak_maxine::{Architecture, Kind};
use bukospeak_maxine::{Effect, Runtime};

use crate::install::{self, Progress};

/// The system's RTX card, if it has one the effects run on.
pub fn card() -> Option<Architecture> {
    bukospeak_maxine::detect().unwrap_or_else(|error| {
        tracing::warn!("finding an RTX card: {error}");
        None
    })
}

/// The folder the effects for `architecture` are installed in, if the system has a folder for
/// programs' data.
fn folder(architecture: Architecture) -> Option<PathBuf> {
    dirs::data_local_dir().map(|folder| folder.join("bukospeak").join("nvidia").join(architecture.name()))
}

/// NVIDIA's effects, installed for a card.
pub struct Effects {
    folder: PathBuf,
    architecture: Architecture,
    /// The runtime, once an effect was made; shared by those made after.
    runtime: Mutex<Option<Arc<Runtime>>>,
    /// Hears that an effect couldn't be made, and the built-in suppression runs instead.
    failed: Box<dyn Fn() + Send + Sync>,
}

impl Effects {
    /// The effects installed for `architecture`, if they are; `failed` hears whenever one
    /// can't be made.
    pub fn installed(architecture: Architecture, failed: impl Fn() + Send + Sync + 'static) -> Option<Arc<Self>> {
        let folder = folder(architecture)?;
        install::is_installed(&folder).then(|| Arc::new(Self { folder, architecture, runtime: Mutex::default(), failed: Box::new(failed) }))
    }

    /// The engine's suppression by the effect `kind`.
    pub fn suppression(self: &Arc<Self>, kind: Kind) -> Suppression {
        Suppression::Plugged(Arc::new(Maker { effects: Arc::clone(self), kind }))
    }

    fn make(&self, kind: Kind) -> Result<Effect, bukospeak_maxine::Error> {
        let runtime = {
            let mut runtime = self.runtime.lock().expect("the runtime lock is never poisoned");
            match &*runtime {
                Some(loaded) => Arc::clone(loaded),
                None => Arc::clone(runtime.insert(Arc::new(Runtime::load(&self.folder)?))),
            }
        };
        Effect::new(&runtime, kind, self.architecture)
    }
}

struct Maker {
    effects: Arc<Effects>,
    kind: Kind,
}

impl MakeSuppressor for Maker {
    fn make(&self) -> Option<Box<dyn Suppressor>> {
        match self.effects.make(self.kind) {
            Ok(effect) => Some(Box::new(Nvidia { effect, delay: self.kind.delay(), failing: false })),
            Err(error) => {
                tracing::warn!("making NVIDIA's {}: {error}", self.kind.name());
                (self.effects.failed)();
                None
            }
        }
    }
}

/// An effect of NVIDIA's, suppressing a call's noise.
struct Nvidia {
    effect: Effect,
    delay: usize,
    /// Whether the last frame failed, told once until one runs again.
    failing: bool,
}

impl Suppressor for Nvidia {
    fn delay(&self) -> usize {
        self.delay
    }

    fn process(&mut self, frame: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]) {
        match self.effect.run(frame, out) {
            Ok(()) => self.failing = false,
            Err(error) => {
                if !std::mem::replace(&mut self.failing, true) {
                    tracing::warn!("running NVIDIA's effect: {error}");
                }
                // Better the voice with its noise, and a little early, than none.
                out.copy_from_slice(frame);
            }
        }
    }

    fn reset(&mut self) {
        if let Err(error) = self.effect.reset() {
            tracing::warn!("resetting NVIDIA's effect: {error}");
        }
    }
}

/// Installs the effects for `architecture` from the download `server` offers, as
/// [`install::install`] does, unless they are installed.
pub fn install(server: &str, architecture: Architecture, progress: impl FnMut(Progress)) -> Result<(), install::Error> {
    let folder = folder(architecture).ok_or(install::Error::NoFolder)?;
    if install::is_installed(&folder) {
        return Ok(());
    }
    install::install(server, &architecture.download(), &folder, progress)
}
