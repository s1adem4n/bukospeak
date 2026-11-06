//! What the window asks of the system it runs on, beside the call: where the settings and the
//! room key are kept, the audio devices, the RTX card and NVIDIA's effects for it, and the
//! server's sounds. The app asks
//! the system itself, through [`Desktop`]; the harness stands in for it.

use std::path::PathBuf;

use crate::audio::{self, Device, Direction};
use crate::install::{self, Progress};
use crate::known_servers::KnownServers;
use crate::nvidia::{self, Architecture};
use crate::settings::{self, Settings};
use crate::{autostart, cues};

pub trait Host: Send + Sync + 'static {
    fn save_settings(&self, settings: &Settings) -> Result<(), settings::Error>;

    /// The room key, if one was kept.
    fn room_key(&self) -> Result<Option<String>, settings::Error>;

    /// Keeps the room key; an empty one removes it.
    fn set_room_key(&self, key: &str) -> Result<(), settings::Error>;

    /// The devices there are for a direction.
    fn devices(&self, direction: Direction) -> Result<Vec<Device>, audio::Error>;

    /// Has the system start the app with it, in the background, or not.
    fn start_with_system(&self, on: bool) -> Result<(), autostart::Error>;

    /// The programs running, the largest first, which a game is among while it runs.
    fn programs(&self) -> Vec<String>;

    /// The system's RTX card, if it has one NVIDIA's effects run on.
    fn card(&self) -> Option<Architecture>;

    /// Installs NVIDIA's effects for `architecture`, as [`nvidia::install`] does; blocks until
    /// they are.
    fn install(
        &self,
        server: &str,
        room_key: String,
        known: &KnownServers,
        architecture: Architecture,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), install::Error>;

    /// Installs the sounds the server offers, as [`cues::sync`] does; blocks until they are.
    fn sync_sounds(&self, server: &str, room_key: String, known: &KnownServers) -> Result<(), install::Error>;
}

/// The system the app runs on, its settings kept in the file at a path.
pub struct Desktop {
    settings: PathBuf,
}

impl Desktop {
    pub fn new(settings: PathBuf) -> Self {
        Self { settings }
    }
}

impl Host for Desktop {
    fn save_settings(&self, settings: &Settings) -> Result<(), settings::Error> {
        settings.save(&self.settings)
    }

    fn room_key(&self) -> Result<Option<String>, settings::Error> {
        settings::room_key()
    }

    fn set_room_key(&self, key: &str) -> Result<(), settings::Error> {
        settings::set_room_key(key)
    }

    fn devices(&self, direction: Direction) -> Result<Vec<Device>, audio::Error> {
        audio::devices(direction)
    }

    fn start_with_system(&self, on: bool) -> Result<(), autostart::Error> {
        // A build of one's own runs from where it was built, which the system isn't to start.
        if cfg!(debug_assertions) {
            tracing::info!("a debug build doesn't start with the system");
            return Ok(());
        }
        autostart::set(on)
    }

    fn programs(&self) -> Vec<String> {
        crate::games::running().unwrap_or_else(|error| {
            tracing::warn!("{error}");
            Vec::new()
        })
    }

    fn card(&self) -> Option<Architecture> {
        nvidia::card()
    }

    fn install(
        &self,
        server: &str,
        room_key: String,
        known: &KnownServers,
        architecture: Architecture,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), install::Error> {
        nvidia::install(server, room_key, known, architecture, progress)
    }

    fn sync_sounds(&self, server: &str, room_key: String, known: &KnownServers) -> Result<(), install::Error> {
        let folder = cues::folder().ok_or(install::Error::NoFolder)?;
        cues::sync(server, room_key, known, &folder)
    }
}
