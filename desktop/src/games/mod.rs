use std::cmp::Reverse;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use gpui::RenderImage;
use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(windows)]
#[allow(unsafe_code, reason = "Windows tells a program's name and icon only through its C interface")]
mod windows;

pub const LOOK: Duration = Duration::from_secs(3);

pub const FORTNITE: &str = "FortniteClient-Win64-Shipping.exe";

const KNOWN: &[(&str, &str)] = &[(FORTNITE, "Fortnite")];

/// The folders launchers install games in, each as the run of folders a game's path passes
/// through; the folder after it is the game's own.
const LIBRARIES: &[&[&str]] = &[
    &["steamapps", "common"],
    &["Epic Games"],
    &["Riot Games"],
    &["XboxGames"],
    &["EA Games"],
    &["Ubisoft Game Launcher", "games"],
    &["GOG Galaxy", "Games"],
];

/// The launchers' own folders in their libraries, which are no game.
const LAUNCHERS: &[&str] = &["Launcher", "Riot Client"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the processes: {0}")]
    Processes(#[from] sysinfo::Error),
    #[error("the watcher's thread: {0}")]
    Thread(#[from] std::io::Error),
}

/// A program, running or chosen to join the call when it runs.
#[derive(Clone)]
pub struct Program {
    /// What the settings keep of it: its path, or only its file name where that is all known.
    pub game: String,
    pub title: String,
    /// Whether it lies in a launcher's library of games.
    pub in_library: bool,
    pub icon: Option<Arc<RenderImage>>,
}

/// A game chosen to join the call when it runs, and whether it runs.
#[derive(Clone)]
pub struct Chosen {
    pub program: Program,
    pub runs: bool,
}

/// The games chosen, and the other programs running, to choose from.
#[derive(Clone, Default)]
pub struct Listing {
    pub chosen: Vec<Chosen>,
    pub others: Vec<Program>,
}

impl Listing {
    /// Chooses `game` from the programs running, as the next listing will have it.
    pub fn choose(&mut self, game: &str) {
        if let Some(at) = self.others.iter().position(|program| program.game == game) {
            let program = self.others.remove(at);
            self.chosen.push(Chosen { program, runs: true });
        }
    }

    /// Lets `game` go, back among the programs running if it runs, as the next listing will have
    /// it.
    pub fn unchoose(&mut self, game: &str) {
        if let Some(at) = self.chosen.iter().position(|chosen| chosen.program.game == game) {
            let Chosen { program, runs } = self.chosen.remove(at);
            if runs {
                self.others.insert(0, program);
            }
        }
    }
}

/// The games of `chosen` as `running` has them, else as `kept` tells them, and the programs of
/// `running` not chosen.
pub fn listing(chosen: &[String], running: Vec<Program>, mut kept: impl FnMut(&str) -> Program) -> Listing {
    let is_running = |game: &str, program: &Program| is(game, Path::new(&program.game));
    let listed = chosen
        .iter()
        .map(|game| match running.iter().find(|program| is_running(game, program)) {
            // Kept as the settings keep it, so it is removed as it was chosen.
            Some(program) => Chosen { program: Program { game: game.clone(), ..program.clone() }, runs: true },
            None => Chosen { program: kept(game), runs: false },
        })
        .collect();
    let others = running.into_iter().filter(|program| !chosen.iter().any(|game| is_running(game, program))).collect();
    Listing { chosen: listed, others }
}

/// Whether `game`, as the settings keep it, is the program at `path`: the same path, or the
/// same file name for a game kept by its file name alone.
pub fn is(game: &str, path: &Path) -> bool {
    let kept = Path::new(game);
    let there = if has_folder(kept) { path.as_os_str() } else { path.file_name().unwrap_or_default() };
    kept.as_os_str().eq_ignore_ascii_case(there)
}

fn has_folder(path: &Path) -> bool {
    path.parent().is_some_and(|folder| !folder.as_os_str().is_empty())
}

/// The game's own folder, if `path` lies in a launcher's library.
fn library_game(path: &Path) -> Option<&OsStr> {
    let folders: Vec<&OsStr> = path.parent()?.iter().collect();
    LIBRARIES.iter().find_map(|library| {
        let at = folders
            .windows(library.len())
            .position(|run| run.iter().zip(*library).all(|(folder, name)| folder.eq_ignore_ascii_case(name)))?;
        let game = *folders.get(at + library.len())?;
        (!LAUNCHERS.iter().any(|launcher| game.eq_ignore_ascii_case(launcher))).then_some(game)
    })
}

fn known(file: &OsStr) -> Option<&'static str> {
    KNOWN.iter().find(|(known, _)| file.eq_ignore_ascii_case(known)).map(|(_, title)| *title)
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

/// The program a process runs: its path, or its name where the system doesn't tell the path.
fn program(process: &Process) -> PathBuf {
    process.exe().map_or_else(|| PathBuf::from(process.name()), Path::to_path_buf)
}

/// What a program's file says of it, read once per path.
#[derive(Clone, Default)]
struct Description {
    title: Option<String>,
    icon: Option<Arc<RenderImage>>,
}

#[cfg(windows)]
fn describe(path: &Path) -> Description {
    use image::{Frame, RgbaImage};

    let icon = windows::icon(path).and_then(|icon| RgbaImage::from_raw(icon.width, icon.height, icon.bgra));
    Description { title: windows::title(path), icon: icon.map(|bgra| Arc::new(RenderImage::new([Frame::new(bgra)]))) }
}

#[cfg(not(windows))]
fn describe(_: &Path) -> Description {
    Description::default()
}

/// The programs that run, and those chosen, as the system and their files tell them.
#[derive(Default)]
pub struct Programs {
    described: HashMap<PathBuf, Description>,
}

impl Programs {
    /// The games of `chosen`, as the settings keep them, and the other programs running.
    pub fn list(&mut self, chosen: &[String]) -> Result<Listing, Error> {
        let running = self.running()?;
        Ok(listing(chosen, running, |game| self.kept(game)))
    }

    /// The programs running, the one taking the most memory first: of a game, its process taking
    /// the most; of another program, which may run as several processes, the same.
    fn running(&mut self) -> Result<Vec<Program>, Error> {
        let mut system = System::new()?;
        let refresh = ProcessRefreshKind::nothing().with_memory().with_exe(UpdateKind::Always);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
        let own = std::env::current_exe().ok();
        let system_folder = std::env::var_os("SystemRoot").map(PathBuf::from);
        let mut largest: HashMap<String, (PathBuf, u64)> = HashMap::new();
        for process in system.processes().values() {
            let Some(path) = process.exe() else { continue };
            if own.as_deref() == Some(path) || system_folder.as_ref().is_some_and(|folder| path.starts_with(folder)) {
                continue;
            }
            let key = match library_game(path) {
                Some(game) => format!("game {}", game.to_string_lossy().to_lowercase()),
                None => path.file_name().unwrap_or_default().to_string_lossy().to_lowercase(),
            };
            let entry = largest.entry(key).or_insert_with(|| (path.to_path_buf(), 0));
            if process.memory() >= entry.1 {
                *entry = (path.to_path_buf(), process.memory());
            }
        }
        let mut programs: Vec<(PathBuf, u64)> = largest.into_values().collect();
        programs.sort_by_key(|(_, memory)| Reverse(*memory));
        Ok(programs.into_iter().map(|(path, _)| self.program(&path)).collect())
    }

    /// `game`, as the settings keep it, when it doesn't run.
    fn kept(&mut self, game: &str) -> Program {
        let path = Path::new(game);
        if has_folder(path) {
            return self.program(path);
        }
        let title = known(path.as_os_str()).map_or_else(|| stem(path), str::to_owned);
        Program { game: game.to_owned(), title, in_library: false, icon: None }
    }

    fn program(&mut self, path: &Path) -> Program {
        let description = self.described.entry(path.to_path_buf()).or_insert_with(|| describe(path)).clone();
        let game = library_game(path);
        let title = path
            .file_name()
            .and_then(known)
            .map(str::to_owned)
            .or(description.title)
            .or_else(|| game.map(|game| game.to_string_lossy().into_owned()))
            .unwrap_or_else(|| stem(path));
        Program { game: path.to_string_lossy().into_owned(), title, in_library: game.is_some(), icon: description.icon }
    }
}

fn any_runs(games: &[String], programs: &[PathBuf]) -> bool {
    programs.iter().any(|program| games.iter().any(|game| is(game, program)))
}

pub struct Watcher {
    games: mpsc::Sender<Vec<String>>,
}

impl Watcher {
    pub fn new(games: Vec<String>, runs: impl Fn(bool) + Send + 'static) -> Result<Self, Error> {
        let (sender, received) = mpsc::channel();
        let mut system = System::new()?;
        thread::Builder::new().name("bukospeak-games".into()).spawn(move || {
            let mut games = games;
            let mut told = None;
            loop {
                let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet);
                system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
                let programs: Vec<PathBuf> = system.processes().values().map(program).collect();
                let now = any_runs(&games, &programs);
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

    pub fn set(&self, games: Vec<String>) {
        // The thread runs as long as this.
        let _ = self.games.send(games);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(folders: &[&str]) -> PathBuf {
        let mut path = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
        path.extend(folders);
        path
    }

    #[test]
    fn a_game_kept_by_its_file_runs_from_anywhere_and_one_kept_by_its_path_from_there() {
        let fortnite = path(&["Epic Games", "Fortnite", "Win64", FORTNITE]);
        assert!(is(FORTNITE, &fortnite));
        assert!(is("fortniteclient-win64-shipping.exe", &fortnite));
        assert!(is(&fortnite.to_string_lossy(), &fortnite));
        assert!(!is(&path(&["Elsewhere", FORTNITE]).to_string_lossy(), &fortnite));
        assert!(any_runs(&[FORTNITE.to_owned()], &[path(&["explorer.exe"]), fortnite]));
        assert!(!any_runs(&[], &[path(&["explorer.exe"])]));
    }

    #[test]
    fn a_game_lies_in_its_own_folder_of_a_launcher_s_library() {
        let game = path(&["Games", "SteamLibrary", "steamapps", "common", "Counter-Strike", "game", "cs2.exe"]);
        assert_eq!(library_game(&game), Some(OsStr::new("Counter-Strike")));
        assert_eq!(library_game(&path(&["Riot Games", "VALORANT", "live", "VALORANT.exe"])), Some(OsStr::new("VALORANT")));
        assert_eq!(library_game(&path(&["Epic Games", "Launcher", "Portal", "EpicGamesLauncher.exe"])), None);
        assert_eq!(library_game(&path(&["steamapps", "common", "loose.exe"])), None);
        assert_eq!(library_game(&path(&["Program Files", "Spotify", "Spotify.exe"])), None);
    }

    #[test]
    fn a_game_s_title_is_its_known_name_else_its_folder_else_its_file() {
        let mut programs = Programs::default();
        assert_eq!(programs.kept(FORTNITE).title, "Fortnite");
        assert_eq!(programs.kept("Game.exe").title, "Game");
        let in_library = programs.kept(&path(&["Riot Games", "VALORANT", "live", "VALORANT-Win64.exe"]).to_string_lossy());
        assert_eq!((in_library.title.as_str(), in_library.in_library), ("VALORANT", true));
        assert_eq!(programs.kept(&path(&["Tools", "tool.exe"]).to_string_lossy()).title, "tool");
    }

    #[test]
    fn a_game_chosen_or_let_go_moves_between_the_lists_as_the_next_listing_has_it() {
        let program = |game: &str| Program { game: game.to_owned(), title: game.to_owned(), in_library: true, icon: None };
        let running = vec![program("first.exe"), program("second.exe")];
        let chosen = |listing: &Listing| listing.chosen.iter().map(|chosen| chosen.program.game.clone()).collect::<Vec<_>>();
        let others = |listing: &Listing| listing.others.iter().map(|program| program.game.clone()).collect::<Vec<_>>();
        let mut listing = listing(&[], running.clone(), program);
        listing.choose("second.exe");
        let relisted = super::listing(&["second.exe".to_owned()], running, program);
        assert_eq!((chosen(&listing), others(&listing)), (chosen(&relisted), others(&relisted)));
        listing.unchoose("second.exe");
        assert_eq!((chosen(&listing), others(&listing)), (vec![], vec!["second.exe".to_owned(), "first.exe".to_owned()]));
    }
}
