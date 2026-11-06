//! The games the app joins the call for: whether one runs, looked at every few seconds on a
//! thread of its own, and what runs now that could be one. A game is known by the file name of
//! its program, as Windows and the others name its process.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// How often the watcher looks: a game takes longer than this to start.
const LOOK: Duration = Duration::from_secs(3);

/// Fortnite's program, which the app joins for unless told otherwise.
pub const FORTNITE: &str = "FortniteClient-Win64-Shipping.exe";

/// Games by their programs, and what they are called.
const KNOWN: &[(&str, &str)] = &[(FORTNITE, "Fortnite")];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the processes: {0}")]
    Processes(#[from] sysinfo::Error),
    #[error("the watcher's thread: {0}")]
    Thread(#[from] std::io::Error),
}

/// How many programs [`running`] offers at most.
const OFFERED: usize = 8;

/// What a game's program is called: its game's name if known, else the file's without its
/// extension.
pub fn title(program: &str) -> &str {
    KNOWN
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(program))
        .map_or_else(|| program.rsplit_once('.').map_or(program, |(stem, _)| stem), |(_, title)| title)
}

/// The programs running, by their file names, the largest first, which a game is among while it
/// runs; the app's own left out.
pub fn running() -> Result<Vec<String>, Error> {
    let mut system = System::new()?;
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory().with_exe(UpdateKind::Always),
    );
    let own = std::env::current_exe().ok().and_then(|path| path.file_name().map(OsStr::to_os_string));
    // A program runs as several processes at times: each counts with its largest.
    let mut largest: BTreeMap<String, u64> = BTreeMap::new();
    for process in system.processes().values() {
        let Some(file) = program(process) else { continue };
        if own.as_deref() == Some(OsStr::new(&file)) {
            continue;
        }
        let memory = largest.entry(file).or_default();
        *memory = (*memory).max(process.memory());
    }
    let mut programs: Vec<(String, u64)> = largest.into_iter().collect();
    programs.sort_by_key(|&(_, memory)| std::cmp::Reverse(memory));
    Ok(programs.into_iter().take(OFFERED).map(|(file, _)| file).collect())
}

/// The file name of a process's program; its own name where its program isn't readable, as
/// another user's isn't.
fn program(process: &sysinfo::Process) -> Option<String> {
    let name = process.exe().and_then(|path| path.file_name()).unwrap_or_else(|| process.name());
    let name = name.to_string_lossy();
    (!name.is_empty()).then(|| name.into_owned())
}

/// Whether one of `games` is among `programs`, by their file names, case aside.
fn any_runs<'a>(games: &[String], mut programs: impl Iterator<Item = &'a str>) -> bool {
    programs.any(|program| games.iter().any(|game| game.eq_ignore_ascii_case(program)))
}

/// Looks every few seconds whether one of its games runs, and tells each time that changes; it
/// stops when dropped.
pub struct Watcher {
    games: mpsc::Sender<Vec<String>>,
}

impl Watcher {
    /// Watches for `games`, telling `runs` whether one does, first right away.
    pub fn new(games: Vec<String>, runs: impl Fn(bool) + Send + 'static) -> Result<Self, Error> {
        let (sender, received) = mpsc::channel();
        let mut system = System::new()?;
        thread::Builder::new().name("bukospeak-games".into()).spawn(move || {
            let mut games = games;
            let mut told = None;
            loop {
                let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet);
                system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
                let names: Vec<String> = system.processes().values().filter_map(program).collect();
                let now = !games.is_empty() && any_runs(&games, names.iter().map(String::as_str));
                if told != Some(now) {
                    told = Some(now);
                    runs(now);
                }
                match received.recv_timeout(LOOK) {
                    Ok(changed) => games = changed,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        })?;
        Ok(Self { games: sender })
    }

    /// Watches for `games` from now on.
    pub fn set(&self, games: Vec<String>) {
        // The thread ends only when this drops.
        let _ = self.games.send(games);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_game_is_called_by_its_name_and_another_by_its_file() {
        assert_eq!(title(FORTNITE), "Fortnite");
        assert_eq!(title("fortniteclient-win64-shipping.exe"), "Fortnite");
        assert_eq!(title("Game.exe"), "Game");
        assert_eq!(title("game"), "game");
    }

    #[test]
    fn a_game_runs_whatever_the_case_of_its_file() {
        let games = vec![FORTNITE.to_owned()];
        assert!(any_runs(&games, ["explorer.exe", "FORTNITECLIENT-WIN64-SHIPPING.EXE"].into_iter()));
        assert!(!any_runs(&games, ["explorer.exe"].into_iter()));
        assert!(!any_runs(&[], ["explorer.exe"].into_iter()));
    }
}
