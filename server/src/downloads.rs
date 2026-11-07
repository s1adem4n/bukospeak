use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind, Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use bukospeak_proto::control::{self, File, FileRequest, Rejection, ServerMessage};
use quinn::{Connection, ConnectionError, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncSeekExt;
use tokio::sync::watch;

use crate::room::reject;

// Written last, so a folder that holds it was prepared whole.
const LIST: &str = ".files.json";

const PROGRESS_EVERY: Duration = Duration::from_millis(250);

const ZSTD_LEVEL: i32 = 3;

pub trait Source: Send + Sync + 'static {
    // A name a source offers must be safe as a folder's name.
    fn offers(&self, name: &str) -> anyhow::Result<Option<String>>;

    fn prepare(&self, name: &str, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()>;
}

#[derive(Clone, Debug)]
enum State {
    Preparing { done: u64, total: u64 },
    Ready(Arc<[File]>),
    Failed,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Prepared {
    stamp: String,
    files: Vec<File>,
}

pub struct Downloads {
    folder: PathBuf,
    sources: Vec<Arc<dyn Source>>,
    states: Mutex<HashMap<String, (String, watch::Receiver<State>)>>,
}

impl Downloads {
    pub fn new(folder: PathBuf, sources: Vec<Arc<dyn Source>>) -> Self {
        Self { folder, sources, states: Mutex::default() }
    }

    pub fn none() -> Self {
        Self::new(PathBuf::new(), Vec::new())
    }

    pub(crate) async fn serve(&self, name: &str, connection: &Connection, mut send: SendStream) -> anyhow::Result<()> {
        let mut state = match self.state(name) {
            Ok(Some(state)) => state,
            Ok(None) => return reject(send, connection, Rejection::Unavailable).await,
            Err(error) => {
                tracing::warn!(name, "offering a download: {error:#}");
                return reject(send, connection, Rejection::Unavailable).await;
            }
        };
        let files = loop {
            let now = state.borrow_and_update().clone();
            match now {
                State::Preparing { done, total } => control::write(&mut send, &ServerMessage::Preparing { done, total }).await?,
                State::Ready(files) => break files,
                State::Failed => return reject(send, connection, Rejection::Unavailable).await,
            }
            tokio::time::sleep(PROGRESS_EVERY).await;
            // The preparation tells it ready or failed before it drops the sender.
            state.changed().await?;
        };
        control::write(&mut send, &ServerMessage::Files { files: files.to_vec() }).await?;
        let folder = self.folder.join(name);
        loop {
            let (send, recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                // The client closes the connection once it has what it wants.
                Err(ConnectionError::ApplicationClosed(_)) => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            let (folder, files) = (folder.clone(), Arc::clone(&files));
            tokio::spawn(async move {
                if let Err(error) = send_file(&folder, &files, send, recv).await {
                    tracing::debug!(%error, "sending a file ended");
                }
            });
        }
    }

    fn state(&self, name: &str) -> anyhow::Result<Option<watch::Receiver<State>>> {
        let mut offered = None;
        for source in &self.sources {
            if let Some(stamp) = source.offers(name)? {
                offered = Some((source, stamp));
                break;
            }
        }
        let Some((source, stamp)) = offered else { return Ok(None) };
        let mut states = self.states.lock().expect("the states lock is never poisoned");
        if let Some((prepared_from, state)) = states.get(name) {
            match *state.borrow() {
                // Even if its source changed meanwhile: the first ask once it is ready prepares it anew.
                State::Preparing { .. } => return Ok(Some(state.clone())),
                State::Ready(_) if *prepared_from == stamp => return Ok(Some(state.clone())),
                State::Ready(_) | State::Failed => {}
            }
        }
        let folder = self.folder.join(name);
        let state = match read_list(&folder)? {
            Some(prepared) if prepared.stamp == stamp => watch::Sender::new(State::Ready(prepared.files.into())).subscribe(),
            _ => {
                let (tell, state) = watch::channel(State::Preparing { done: 0, total: 0 });
                let (source, folder, name, stamp) = (Arc::clone(source), self.folder.clone(), name.to_owned(), stamp.clone());
                tokio::task::spawn_blocking(move || {
                    tracing::info!(name, "preparing a download");
                    let progress = |done, total| {
                        tell.send_replace(State::Preparing { done, total });
                    };
                    match prepare(source.as_ref(), &name, stamp, &folder, &progress) {
                        Ok(files) => {
                            tracing::info!(name, "prepared a download");
                            tell.send_replace(State::Ready(files.into()));
                        }
                        Err(error) => {
                            tracing::warn!(name, "preparing a download: {error:#}");
                            tell.send_replace(State::Failed);
                        }
                    }
                });
                state
            }
        };
        states.insert(name.to_owned(), (stamp, state.clone()));
        Ok(Some(state))
    }
}

fn prepare(source: &dyn Source, name: &str, stamp: String, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<Vec<File>> {
    let preparing = folder.join(format!(".{name}"));
    remove_folder(&preparing)?;
    fs::create_dir_all(&preparing).with_context(|| format!("creating {}", preparing.display()))?;
    source.prepare(name, &preparing, progress)?;
    let mut files = Vec::new();
    list(&preparing, "", &mut files)?;
    let prepared = Prepared { stamp, files };
    let text = serde_json::to_string(&prepared).expect("a list of files is plain text");
    fs::write(preparing.join(LIST), text).context("writing the list of files")?;
    let ready = folder.join(name);
    remove_folder(&ready)?;
    fs::rename(&preparing, &ready).with_context(|| format!("moving the download to {}", ready.display()))?;
    Ok(prepared.files)
}

fn list(folder: &Path, prefix: &str, files: &mut Vec<File>) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(folder)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|name| anyhow::anyhow!("{name:?} isn't UTF-8"))?;
        let path = format!("{prefix}{name}");
        if entry.file_type()?.is_dir() {
            list(&entry.path(), &format!("{path}/"), files)?;
        } else {
            let (size, sha256) = control::digest(&mut fs::File::open(entry.path())?)?;
            files.push(File { path, size, sha256 });
        }
    }
    Ok(())
}

fn read_list(folder: &Path) -> anyhow::Result<Option<Prepared>> {
    let text = match fs::read_to_string(folder.join(LIST)) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading the list of files"),
    };
    match serde_json::from_str(&text) {
        Ok(prepared) => Ok(Some(prepared)),
        Err(error) => {
            tracing::info!("{} isn't this server's list of files, so it is prepared anew: {error}", folder.display());
            Ok(None)
        }
    }
}

fn remove_folder(folder: &Path) -> anyhow::Result<()> {
    match fs::remove_dir_all(folder) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error).with_context(|| format!("removing {}", folder.display())),
        _ => Ok(()),
    }
}

pub fn compress(from: &mut (impl Read + ?Sized), to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    zstd::stream::copy_encode(from, fs::File::create(to)?, ZSTD_LEVEL)
}

async fn send_file(folder: &Path, files: &[File], mut send: SendStream, mut recv: RecvStream) -> anyhow::Result<()> {
    let request: FileRequest = control::read(&mut recv).await?.context("the stream ended before a request")?;
    // Only a listed path reaches the file system.
    let file = files.iter().find(|file| file.path == request.path).context("the download has no such file")?;
    let mut opened = tokio::fs::File::open(folder.join(&file.path)).await?;
    opened.seek(SeekFrom::Start(request.from)).await?;
    tokio::io::copy(&mut opened, &mut send).await?;
    send.finish()?;
    send.stopped().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Two;

    impl Source for Two {
        fn offers(&self, name: &str) -> anyhow::Result<Option<String>> {
            Ok((name == "first").then(|| "stamp".into()))
        }

        fn prepare(&self, _: &str, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
            compress(&mut &b"one"[..], &folder.join("one"))?;
            progress(1, 2);
            compress(&mut &[7u8; 10_000][..], &folder.join("sub/two"))?;
            progress(2, 2);
            Ok(())
        }
    }

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("bukospeak-downloads-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_prepared_download_lists_its_files_compressed_and_is_ready_after() {
        let folder = folder("prepared");
        let files = prepare(&Two, "first", "stamp".into(), &folder, &|_, _| ()).unwrap();
        assert_eq!(files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), ["one", "sub/two"]);
        let compressed = fs::read(folder.join("first/sub/two")).unwrap();
        assert_eq!((files[1].size, files[1].sha256.clone()), control::digest(&mut compressed.as_slice()).unwrap());
        assert_eq!(zstd::decode_all(compressed.as_slice()).unwrap(), [7u8; 10_000]);
        assert_eq!(read_list(&folder.join("first")).unwrap().unwrap(), Prepared { stamp: "stamp".into(), files });
        assert!(!folder.join(".first").exists());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_download_prepared_in_part_isnt_ready() {
        let folder = folder("part");
        fs::create_dir_all(folder.join("first")).unwrap();
        fs::write(folder.join("first/one"), b"one").unwrap();
        assert_eq!(read_list(&folder.join("first")).unwrap(), None);
        fs::remove_dir_all(folder).unwrap();
    }
}
