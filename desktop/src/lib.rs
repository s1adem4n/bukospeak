//! What the BukoSpeak desktop app and the terminal client share.

pub mod known_servers;

use std::path::PathBuf;

/// The folder the desktop clients keep their files in, if the system has a config folder.
pub fn config_folder() -> Option<PathBuf> {
    dirs::config_dir().map(|folder| folder.join("bukospeak"))
}
