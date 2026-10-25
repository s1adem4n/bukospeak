//! What the BukoSpeak desktop app and the terminal client share.

pub mod call;
pub mod known_servers;
pub mod settings;

use std::path::{Path, PathBuf};
use std::{fs, io, process};

/// The folder the desktop clients keep their files in, if the system has a config folder.
pub fn config_folder() -> Option<PathBuf> {
    dirs::config_dir().map(|folder| folder.join("bukospeak"))
}

/// Writes `text` to a file of its own beside `path` and renames it over it, so a reader never
/// sees half of it; makes the folder if there is none.
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
