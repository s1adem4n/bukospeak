pub mod audio;
pub mod autopilot;
pub mod autostart;
pub mod call;
pub mod cues;
pub mod games;
#[cfg(feature = "harness")]
pub mod harness;
pub mod nvidia;
pub mod settings;
pub mod ui;
pub mod update;

use std::path::{Path, PathBuf};
use std::{fs, io, process};

// PulseAudio's client logs an error each time a connection closes as it should, after listing devices.
pub const LOG_FILTER: &str = "warn,pulseaudio::client::reactor=off";

pub fn config_folder() -> Option<PathBuf> {
    dirs::config_dir().map(|folder| folder.join("bukospeak"))
}

fn replace(path: &Path, text: &str) -> io::Result<()> {
    let folder = path.parent().expect("the file is inside a folder");
    let name = path.file_name().expect("the path names a file").to_string_lossy();
    let written = folder.join(format!(".{name}.{}", process::id()));
    fs::create_dir_all(folder)?;
    fs::write(&written, text)?;
    fs::rename(&written, path).inspect_err(|_| {
        // The rename's error is the one to tell; the stray file only takes space.
        let _ = fs::remove_file(&written);
    })
}
