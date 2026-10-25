//! What the user set up: who they are, where the channel is and how the call sounds, in
//! `settings.toml`; the room key in the system's keyring (Credential Manager, Keychain, Secret
//! Service).

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use bukospeak_audio_desktop::{DeviceId, Direction};
use serde::{Deserialize, Serialize};

use crate::call::{Devices, Join};

/// Where the room key is kept in the keyring.
const KEYRING_SERVICE: &str = "bukospeak";
const KEYRING_USER: &str = "room-key";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Parse { path: PathBuf, source: toml::de::Error },
    #[error("writing {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("the keyring: {0}")]
    Keyring(#[from] keyring::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    pub name: String,
    /// The server as `host:port`.
    pub server: String,
    /// The microphone's device id as cpal writes it; the system's default for None.
    pub input: Option<String>,
    /// The speakers' or headphones' device id; the system's default for None.
    pub output: Option<String>,
    /// Whether RNNoise suppresses the microphone's noise.
    pub noise_suppression: bool,
    /// Whether the speakers' echo is cancelled in what the microphone hears.
    pub echo_cancellation: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { name: String::new(), server: String::new(), input: None, output: None, noise_suppression: true, echo_cancellation: true }
    }
}

impl Settings {
    /// The settings in the file at `path`; the defaults if there is none yet. A setting the file
    /// lacks takes its default.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| Error::Parse { path: path.to_owned(), source }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read { path: path.to_owned(), source }),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let text = toml::to_string(self).expect("the settings are plain text and switches");
        crate::replace(path, &text).map_err(|source| Error::Write { path: path.to_owned(), source })
    }

    /// Whether there is enough to join, with the room key.
    pub fn is_complete(&self, room_key: &str) -> bool {
        ![self.name.as_str(), self.server.as_str(), room_key].iter().any(|text| text.trim().is_empty())
    }

    /// The join these settings make with the room key.
    pub fn join(&self, room_key: &str) -> Join {
        Join {
            server: self.server.trim().to_owned(),
            name: self.name.trim().to_owned(),
            room_key: room_key.to_owned(),
            devices: Devices { input: self.device(Direction::Input), output: self.device(Direction::Output) },
        }
    }

    /// The id kept for a direction's device; None for the system's default.
    pub fn device_id(&self, direction: Direction) -> Option<&str> {
        match direction {
            Direction::Input => self.input.as_deref(),
            Direction::Output => self.output.as_deref(),
        }
    }

    pub fn set_device_id(&mut self, direction: Direction, id: Option<String>) {
        *match direction {
            Direction::Input => &mut self.input,
            Direction::Output => &mut self.output,
        } = id;
    }

    /// The device kept for a direction; the system's default for none, or for an id that isn't
    /// one here, as one written on another system.
    pub fn device(&self, direction: Direction) -> Option<DeviceId> {
        let id = self.device_id(direction)?;
        id.parse().inspect_err(|error| tracing::warn!("the device {id} isn't one: {error}")).ok()
    }
}

/// The room key, if one was kept.
pub fn room_key() -> Result<Option<String>, Error> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Keeps the room key; an empty one removes it.
pub fn set_room_key(key: &str) -> Result<(), Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match key {
        "" => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        },
        key => Ok(entry.set_password(key)?),
    }
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    /// A fresh folder of its own under the system's temporary one.
    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("bukospeak-settings-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&folder); // Left over from an earlier run, if at all.
        folder
    }

    #[test]
    fn settings_are_kept() {
        let folder = folder("kept");
        let path = folder.join("settings.toml");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let settings = Settings {
            name: "first".into(),
            server: "example.com:9988".into(),
            input: Some("alsa:hw:0,0".into()),
            output: None,
            noise_suppression: false,
            echo_cancellation: true,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_setting_the_file_lacks_takes_its_default() {
        let folder = folder("lacks");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.toml");
        fs::write(&path, "name = \"first\"\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap(), Settings { name: "first".into(), ..Settings::default() });
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_join_takes_the_settings_trimmed_and_the_default_for_a_device_that_isnt_one() {
        let settings =
            Settings { name: " first ".into(), server: "example.com:9988 ".into(), input: Some("nonsense".into()), ..Settings::default() };
        let join = settings.join("key");
        assert_eq!((join.name.as_str(), join.server.as_str(), join.room_key.as_str()), ("first", "example.com:9988", "key"));
        assert_eq!(join.devices, Devices::default());
    }

    #[test]
    fn a_blank_field_is_missing() {
        let settings = Settings { name: "first".into(), server: "example.com:9988".into(), ..Settings::default() };
        assert!(settings.is_complete("key"));
        assert!(!settings.is_complete("  "));
        assert!(!Settings { name: " ".into(), ..settings }.is_complete("key"));
    }
}
