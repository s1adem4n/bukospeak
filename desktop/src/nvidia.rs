//! NVIDIA's effects on an RTX card, which only Windows runs: installed from the download the
//! server offers for the card's generation, and run as the call's noise suppression.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use bukospeak_core::download::Download;
use bukospeak_core::proto::audio::FRAME_SAMPLES;
use bukospeak_core::{MakeSuppressor, Suppression, Suppressor};
pub use bukospeak_maxine::{Architecture, Kind};
use bukospeak_maxine::{Effect, Runtime};

use crate::known_servers::{self, KnownServers};

/// The list of an installed download's files, in its folder, written once all of them are
/// there.
const INSTALLED: &str = "installed.json";

/// Where the files fetched go until they are unpacked, in the download's folder.
const FETCHED: &str = ".fetched";

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
        folder
            .join(INSTALLED)
            .is_file()
            .then(|| Arc::new(Self { folder, architecture, runtime: Mutex::default(), failed: Box::new(failed) }))
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

/// How far an installation got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The server gets the download ready, of which it has `done` of `total` bytes.
    Preparing { done: u64, total: u64 },
    /// `done` of the download's `total` bytes are here.
    Fetching { done: u64, total: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this system has no folder for programs' data")]
    NoFolder,
    #[error(transparent)]
    KnownServers(#[from] known_servers::Error),
    #[error(transparent)]
    Download(#[from] bukospeak_core::Error),
    #[error("the server lists a file outside the download: {0}")]
    Path(String),
    #[error("installing: {0}")]
    Io(#[from] io::Error),
}

/// Installs the effects for `architecture` from the download `server` offers, which the room
/// key opens, unless they are installed; `progress` hears how far it got. Trusts the server by
/// its fingerprint in `known`, or on first use. Blocks until the effects are installed, which
/// may take minutes.
pub fn install(
    server: &str,
    room_key: String,
    known: &KnownServers,
    architecture: Architecture,
    mut progress: impl FnMut(Progress),
) -> Result<(), Error> {
    let folder = folder(architecture).ok_or(Error::NoFolder)?;
    if folder.join(INSTALLED).is_file() {
        return Ok(());
    }
    let pin = known.get(server)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let preparing = |done, total| progress(Progress::Preparing { done, total });
        let download = Download::start(server, room_key, pin, architecture.download(), preparing).await?;
        if pin.is_none() {
            known.insert(server, download.fingerprint())?;
        }
        let files = download.files().to_vec();
        let total = files.iter().map(|file| file.size).sum();
        let mut before = 0;
        for file in &files {
            let to = folder.join(relative(&file.path)?);
            let mut fetched = folder.join(FETCHED).join(relative(&file.path)?).into_os_string();
            fetched.push(".zst");
            let fetched = PathBuf::from(fetched);
            fs::create_dir_all(fetched.parent().expect("a file is inside a folder"))?;
            let fetch = |done| progress(Progress::Fetching { done: before + done, total });
            download.fetch(file, &fetched, fetch).await?;
            before += file.size;
            tokio::task::spawn_blocking(move || unpack(&fetched, &to)).await.expect("unpacking a file doesn't panic")?;
        }
        download.close().await;
        fs::remove_dir_all(folder.join(FETCHED))?;
        let list = serde_json::to_string(&files).expect("a list of files is plain text");
        fs::write(folder.join(INSTALLED), list)?;
        Ok(())
    })
}

/// A path a download lists, as one inside the download's folder.
fn relative(path: &str) -> Result<&Path, Error> {
    let relative = Path::new(path);
    let mut components = relative.components().peekable();
    match components.peek().is_some() && components.all(|component| matches!(component, Component::Normal(_))) {
        true => Ok(relative),
        false => Err(Error::Path(path.to_owned())),
    }
}

/// Unpacks the file the server compressed at `from` to `to`, and removes it.
fn unpack(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to.parent().expect("a file is inside a folder"))?;
    zstd::stream::copy_decode(fs::File::open(from)?, fs::File::create(to)?)?;
    fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_lists_only_paths_inside_its_folder() {
        assert!(relative("bin/NVAudioEffects.dll").is_ok());
        for outside in ["../evil.dll", "/etc/passwd", "bin/../../evil.dll", ""] {
            assert!(relative(outside).is_err(), "{outside}");
        }
    }
}
