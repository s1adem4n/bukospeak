use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::audio::{DeviceId, Direction};
use serde::{Deserialize, Serialize};

use crate::call::{Devices, Join};
use crate::nvidia::{Effects, Kind};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Parse { path: PathBuf, source: toml::de::Error },
    #[error("writing {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    pub server: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub suppression: Suppression,
    pub echo_cancellation: bool,
    pub volume: u16,
    pub start_with_system: bool,
    pub games: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: String::new(),
            input: None,
            output: None,
            suppression: Suppression::Standard,
            echo_cancellation: true,
            volume: 100,
            start_with_system: true,
            games: vec![crate::games::FORTNITE.to_owned()],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Suppression {
    #[value(help = "None: the microphone is sent as captured")]
    Off,
    #[value(help = "The best suppression the engine has built in")]
    Standard,
    #[value(help = "NVIDIA's effect that takes out room echo and noise, on an RTX card")]
    NvidiaDereverbDenoiser,
    #[value(help = "NVIDIA's Studio Voice, which makes the voice sound as if from a studio's microphone, on an RTX card")]
    NvidiaStudioVoice,
}

impl Suppression {
    pub fn nvidia(self) -> Option<Kind> {
        match self {
            Self::Off | Self::Standard => None,
            Self::NvidiaDereverbDenoiser => Some(Kind::DereverbDenoiser),
            Self::NvidiaStudioVoice => Some(Kind::StudioVoiceLowLatency),
        }
    }

    pub fn engines(self, effects: Option<&Arc<Effects>>) -> bukospeak_core::Suppression {
        match (self, self.nvidia(), effects) {
            (Self::Off, ..) => bukospeak_core::Suppression::Off,
            (_, Some(kind), Some(effects)) => effects.suppression(kind),
            _ => bukospeak_core::Suppression::built_in(),
        }
    }
}

impl Settings {
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

    pub fn is_complete(&self) -> bool {
        !self.server.trim().is_empty()
    }

    pub fn server(&self) -> &str {
        self.server.trim()
    }

    pub fn join(&self) -> Join {
        Join {
            server: self.server().to_owned(),
            devices: Devices { input: self.device(Direction::Input), output: self.device(Direction::Output) },
        }
    }

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

    pub fn device(&self, direction: Direction) -> Option<DeviceId> {
        let id = self.device_id(direction)?;
        id.parse().inspect_err(|error| tracing::warn!("the device {id} isn't one: {error}")).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("bukospeak-settings-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn settings_are_kept() {
        let folder = folder("kept");
        let path = folder.join("settings.toml");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let settings = Settings {
            server: "example.com:9988".into(),
            input: Some("alsa:hw:0,0".into()),
            output: None,
            suppression: Suppression::Off,
            echo_cancellation: true,
            volume: 150,
            start_with_system: false,
            games: vec!["Game.exe".into()],
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_setting_the_file_lacks_takes_its_default_and_one_no_longer_had_is_passed_over() {
        let folder = folder("lacks");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.toml");
        fs::write(&path, "name = \"first\"\nserver = \"example.com:9988\"\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap(), Settings { server: "example.com:9988".into(), ..Settings::default() });
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_join_takes_the_settings_trimmed_and_the_default_for_a_device_that_isnt_one() {
        let settings = Settings { server: " example.com:9988 ".into(), input: Some("nonsense".into()), ..Settings::default() };
        let join = settings.join();
        assert_eq!(join.server, "example.com:9988");
        assert_eq!(join.devices, Devices::default());
    }

    #[test]
    fn a_blank_server_is_missing() {
        assert!(Settings { server: "example.com:9988".into(), ..Settings::default() }.is_complete());
        assert!(!Settings { server: "  ".into(), ..Settings::default() }.is_complete());
    }
}
