use std::path::{MAIN_SEPARATOR_STR, PathBuf};
use std::thread;

use crate::audio::{self, Device, DeviceId, Direction};
use crate::autostart;
use crate::games::{self, Listing, Program};
use crate::nvidia::Architecture;
use crate::settings::{self, Settings};
use crate::ui::Host;
use bukospeak_core::install::{self, Progress};

pub struct Staged {
    pub inputs: Vec<(&'static str, &'static str)>,
    pub outputs: Vec<(&'static str, &'static str)>,
    pub card: Option<Architecture>,
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

    fn programs(&self, chosen: &[String]) -> Listing {
        let program = |folders: &[&str], title: &str, in_library| {
            let mut path = PathBuf::from(MAIN_SEPARATOR_STR);
            path.extend(folders);
            Program { game: path.to_string_lossy().into_owned(), title: title.to_owned(), in_library, icon: None }
        };
        let running = vec![
            program(&["Epic Games", "Fortnite", games::FORTNITE], "Fortnite", true),
            program(&["SteamLibrary", "steamapps", "common", "Counter-Strike Global Offensive", "cs2.exe"], "Counter-Strike 2", true),
            program(&["SteamLibrary", "steamapps", "common", "rocketleague", "RocketLeague.exe"], "Rocket League", true),
            program(&["Program Files", "Google", "Chrome", "chrome.exe"], "Google Chrome", false),
            program(&["Program Files", "Spotify", "Spotify.exe"], "Spotify", false),
        ];
        games::listing(chosen, running, |game| program(&[game], game, false))
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
