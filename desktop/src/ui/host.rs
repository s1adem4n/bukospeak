use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::audio::{self, Device, Direction};
use crate::games::{self, Listing};
use crate::install::{self, Progress};
use crate::nvidia::{self, Architecture};
use crate::settings::{self, Settings};
use crate::{autostart, cues};

pub trait Host: Send + Sync + 'static {
    fn save_settings(&self, settings: &Settings) -> Result<(), settings::Error>;

    fn devices(&self, direction: Direction) -> Result<Vec<Device>, audio::Error>;

    fn start_with_system(&self, on: bool) -> Result<(), autostart::Error>;

    /// The games of `chosen`, as the settings keep them, and the other programs running.
    fn programs(&self, chosen: &[String]) -> Listing;

    fn card(&self) -> Option<Architecture>;

    fn install(&self, server: &str, architecture: Architecture, progress: &mut dyn FnMut(Progress)) -> Result<(), install::Error>;

    fn sync_sounds(&self, server: &str) -> Result<(), install::Error>;
}

pub struct Desktop {
    settings: PathBuf,
    programs: Mutex<games::Programs>,
}

impl Desktop {
    pub fn new(settings: PathBuf) -> Self {
        Self { settings, programs: Mutex::default() }
    }
}

impl Host for Desktop {
    fn save_settings(&self, settings: &Settings) -> Result<(), settings::Error> {
        settings.save(&self.settings)
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

    fn programs(&self, chosen: &[String]) -> Listing {
        // What a listing that panicked left is only what files said of programs, still true.
        let mut programs = self.programs.lock().unwrap_or_else(PoisonError::into_inner);
        programs.list(chosen).unwrap_or_else(|error| {
            tracing::warn!("{error}");
            Listing::default()
        })
    }

    fn card(&self) -> Option<Architecture> {
        nvidia::card()
    }

    fn install(&self, server: &str, architecture: Architecture, progress: &mut dyn FnMut(Progress)) -> Result<(), install::Error> {
        nvidia::install(server, architecture, progress)
    }

    fn sync_sounds(&self, server: &str) -> Result<(), install::Error> {
        let folder = cues::folder().ok_or(install::Error::NoFolder)?;
        cues::sync(server, &folder)
    }
}
