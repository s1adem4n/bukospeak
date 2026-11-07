//! The system as a scene gives it: settings kept nowhere, devices and an RTX card as staged,
//! NVIDIA's effects installing without end, and no sounds from a server.

use std::thread;

use crate::audio::{self, Device, DeviceId, Direction};
use crate::autostart;
use crate::install::{self, Progress};
use crate::nvidia::Architecture;
use crate::settings::{self, Settings};
use crate::ui::Host;

pub struct Staged {
    /// The microphones and the outputs, by id and name; the first of each is the default.
    pub inputs: Vec<(&'static str, &'static str)>,
    pub outputs: Vec<(&'static str, &'static str)>,
    pub card: Option<Architecture>,
    /// How far an installation of NVIDIA's effects gets, where it stays.
    pub installing: Progress,
}

impl Host for Staged {
    fn save_settings(&self, _: &Settings) -> Result<(), settings::Error> {
        Ok(())
    }

    fn devices(&self, direction: Direction) -> Result<Vec<Device>, audio::Error> {
        let devices = match direction {
            Direction::Input => &self.inputs,
            Direction::Output => &self.outputs,
        };
        Ok(devices
            .iter()
            .enumerate()
            .map(|(index, (id, name))| Device {
                id: id.parse::<DeviceId>().expect("a staged device's id is one"),
                name: (*name).to_owned(),
                is_default: index == 0,
            })
            .collect())
    }

    fn start_with_system(&self, _: bool) -> Result<(), autostart::Error> {
        Ok(())
    }

    fn programs(&self) -> Vec<String> {
        ["FortniteClient-Win64-Shipping.exe", "chrome.exe", "Spotify.exe"].map(str::to_owned).to_vec()
    }

    fn card(&self) -> Option<Architecture> {
        self.card
    }

    fn install(&self, _: &str, _: Architecture, progress: &mut dyn FnMut(Progress)) -> Result<(), install::Error> {
        progress(self.installing);
        // Stays where it got, as the scene shows it; the harness ends without waiting for it.
        loop {
            thread::park();
        }
    }

    fn sync_sounds(&self, _: &str) -> Result<(), install::Error> {
        Ok(())
    }
}
