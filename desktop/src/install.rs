//! Installing a download the server offers into a folder of the app's: its files fetched, each
//! resumed where a fetch before stopped, unpacked into a folder beside it, which then takes its
//! place, so the folder holds a whole download or none.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use bukospeak_core::download::Download;

/// The list of an installed download's files, in its folder, written once all of them are
/// there.
const INSTALLED: &str = "installed.json";

/// The list of the files the folder being installed is fetching, which it is begun anew without
/// once the server lists others.
const FETCHING: &str = "fetching.json";

/// Where the files fetched go until they are unpacked, in the folder being installed.
const FETCHED: &str = ".fetched";

/// How far an installation got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The server gets the download ready, of which it has `done` of `total` bytes.
    Preparing { done: u64, total: u64 },
    /// `done` of the download's `total` bytes are here.
    Fetching { done: u64, total: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this system has no folder for programs' data")]
    NoFolder,
    #[error(transparent)]
    Download(#[from] bukospeak_core::Error),
    #[error("the server lists a file outside the download: {0}")]
    Path(String),
    #[error("installing: {0}")]
    Io(#[from] io::Error),
}

/// Whether `folder` holds a download installed whole.
pub fn is_installed(folder: &Path) -> bool {
    folder.join(INSTALLED).is_file()
}

/// Installs the download `name` that `server` offers into `folder`, unless the folder holds it
/// as the server lists it now; `progress` hears how far it got. Blocks until it is installed,
/// which may take minutes.
pub fn install(server: &str, name: &str, folder: &Path, mut progress: impl FnMut(Progress)) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let preparing = |done, total| progress(Progress::Preparing { done, total });
        let download = Download::start(server, name.to_owned(), preparing).await?;
        let files = download.files().to_vec();
        let list = serde_json::to_string(&files).expect("a list of files is plain text");
        if read(&folder.join(INSTALLED))?.as_deref() == Some(list.as_str()) {
            download.close().await;
            return Ok(());
        }
        let installing = beside(folder, ".installing");
        // What another list left there is of no use.
        if read(&installing.join(FETCHING))?.as_deref() != Some(list.as_str()) {
            remove(&installing)?;
            fs::create_dir_all(&installing)?;
            fs::write(installing.join(FETCHING), &list)?;
        }
        let total = files.iter().map(|file| file.size).sum();
        let mut before = 0;
        for file in &files {
            let to = installing.join(relative(&file.path)?);
            let mut fetched = installing.join(FETCHED).join(relative(&file.path)?).into_os_string();
            fetched.push(".zst");
            let fetched = PathBuf::from(fetched);
            fs::create_dir_all(fetched.parent().expect("a file is inside a folder"))?;
            let fetch = |done| progress(Progress::Fetching { done: before + done, total });
            download.fetch(file, &fetched, fetch).await?;
            before += file.size;
            tokio::task::spawn_blocking(move || unpack(&fetched, &to)).await.expect("unpacking a file doesn't panic")?;
        }
        download.close().await;
        remove(&installing.join(FETCHED))?;
        fs::remove_file(installing.join(FETCHING))?;
        fs::write(installing.join(INSTALLED), list)?;
        // Gone a moment, the download is never there in part.
        remove(folder)?;
        fs::rename(&installing, folder)?;
        Ok(())
    })
}

/// A path a download lists, as one inside the download's folder.
fn relative(path: &str) -> Result<&Path, Error> {
    let relative = Path::new(path);
    let mut components = relative.components().peekable();
    match components.peek().is_some() && components.all(|component| matches!(component, Component::Normal(_))) {
        true => Ok(relative),
        false => Err(Error::Path(path.to_owned())),
    }
}

/// The folder beside `folder` called as it is, with `suffix`.
fn beside(folder: &Path, suffix: &str) -> PathBuf {
    let mut name = folder.file_name().map_or_else(OsString::new, OsString::from);
    name.push(suffix);
    folder.with_file_name(name)
}

/// The text of the file at `path`, if there is one.
fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Removes `folder` with what is in it, if it is there.
pub fn remove(folder: &Path) -> io::Result<()> {
    match fs::remove_dir_all(folder) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Unpacks the file the server compressed at `from` to `to`, and removes it.
fn unpack(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to.parent().expect("a file is inside a folder"))?;
    zstd::stream::copy_decode(fs::File::open(from)?, fs::File::create(to)?)?;
    fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_lists_only_paths_inside_its_folder() {
        assert!(relative("bin/NVAudioEffects.dll").is_ok());
        for outside in ["../evil.dll", "/etc/passwd", "bin/../../evil.dll", ""] {
            assert!(relative(outside).is_err(), "{outside}");
        }
    }

    #[test]
    fn a_folder_is_installed_beside_where_it_goes() {
        assert_eq!(beside(Path::new("data/sounds"), ".installing"), Path::new("data/sounds.installing"));
    }
}
