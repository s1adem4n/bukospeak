use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::download::Download;

const INSTALLED_LIST: &str = "installed.json";

const FETCHING_LIST: &str = "fetching.json";

const FETCHED_FOLDER: &str = ".fetched";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Preparing { done: u64, total: u64 },
    Fetching { done: u64, total: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this system has no folder for programs' data")]
    NoFolder,
    #[error(transparent)]
    Download(#[from] crate::Error),
    #[error("the server lists a file outside the download: {0}")]
    Path(String),
    #[error("installing: {0}")]
    Io(#[from] io::Error),
}

pub fn is_installed(folder: &Path) -> bool {
    folder.join(INSTALLED_LIST).is_file()
}

pub fn install(server: &str, name: &str, folder: &Path, mut progress: impl FnMut(Progress)) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let preparing = |done, total| progress(Progress::Preparing { done, total });
        let download = Download::start(server, name.to_owned(), preparing).await?;
        let files = download.files().to_vec();
        let list = serde_json::to_string(&files).expect("a list of files is plain text");
        if read(&folder.join(INSTALLED_LIST))?.as_deref() == Some(list.as_str()) {
            download.close().await;
            return Ok(());
        }
        let installing = beside(folder, ".installing");
        // Files fetched for another list are of no use.
        if read(&installing.join(FETCHING_LIST))?.as_deref() != Some(list.as_str()) {
            remove(&installing)?;
            fs::create_dir_all(&installing)?;
            fs::write(installing.join(FETCHING_LIST), &list)?;
        }
        let total = files.iter().map(|file| file.size).sum();
        let mut before = 0;
        for file in &files {
            let to = installing.join(relative(&file.path)?);
            let mut fetched = installing.join(FETCHED_FOLDER).join(relative(&file.path)?).into_os_string();
            fetched.push(".zst");
            let fetched = PathBuf::from(fetched);
            fs::create_dir_all(fetched.parent().expect("a file is inside a folder"))?;
            let fetch = |done| progress(Progress::Fetching { done: before + done, total });
            download.fetch(file, &fetched, fetch).await?;
            before += file.size;
            tokio::task::spawn_blocking(move || unpack(&fetched, &to)).await.expect("unpacking a file doesn't panic")?;
        }
        download.close().await;
        remove(&installing.join(FETCHED_FOLDER))?;
        fs::remove_file(installing.join(FETCHING_LIST))?;
        fs::write(installing.join(INSTALLED_LIST), list)?;
        // The folder is gone a moment but never holds part of a download.
        remove(folder)?;
        fs::rename(&installing, folder)?;
        Ok(())
    })
}

fn relative(path: &str) -> Result<&Path, Error> {
    let relative = Path::new(path);
    let mut components = relative.components().peekable();
    match components.peek().is_some() && components.all(|component| matches!(component, Component::Normal(_))) {
        true => Ok(relative),
        false => Err(Error::Path(path.to_owned())),
    }
}

fn beside(folder: &Path, suffix: &str) -> PathBuf {
    let mut name = folder.file_name().map_or_else(OsString::new, OsString::from);
    name.push(suffix);
    folder.with_file_name(name)
}

fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn remove(folder: &Path) -> io::Result<()> {
    match fs::remove_dir_all(folder) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

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
