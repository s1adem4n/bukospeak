use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bukospeak_core::Error;
use bukospeak_core::download::Download;
use bukospeak_core::proto::control::Rejection;
use bukospeak_server::{Device, Downloads, Room, Source, compress, endpoint};

/// Offers `first`, of a small file and a large one, from what `stamp` says, failing as many
/// times first as `failures` says.
struct Fake {
    prepared: Arc<AtomicU32>,
    stamp: Arc<AtomicU32>,
    failures: u32,
}

const LARGE: usize = 3 << 20;

/// Noise, which zstd can't make much smaller.
fn large() -> Vec<u8> {
    let mut state = 1u32;
    (0..LARGE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

impl Source for Fake {
    fn offers(&self, name: &str) -> anyhow::Result<Option<String>> {
        Ok((name == "first").then(|| self.stamp.load(Ordering::Relaxed).to_string()))
    }

    fn prepare(&self, _: &str, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
        let prepared = self.prepared.fetch_add(1, Ordering::Relaxed);
        progress(0, 2);
        anyhow::ensure!(prepared >= self.failures, "failing as asked");
        compress(&mut &b"small"[..], &folder.join("small"))?;
        progress(1, 2);
        compress(&mut large().as_slice(), &folder.join("sub/large"))?;
        progress(2, 2);
        Ok(())
    }
}

struct Server {
    address: String,
    prepared: Arc<AtomicU32>,
    stamp: Arc<AtomicU32>,
    _folder: Folder,
}

/// A folder of the test's own, removed when it drops.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bukospeak-test-{name}-{}", std::process::id()));
        // Left over from a run that didn't finish, if at all.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // A folder in the system's temporary folder left behind does no harm.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A tailnet in which each connection is a device of its own.
fn own(peer: SocketAddr) -> Option<Device> {
    Some(Device { id: peer.to_string(), name: format!("member {}", peer.port()) })
}

fn serve(name: &str, failures: u32) -> Server {
    let endpoint = endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    let folder = Folder::new(name);
    let (prepared, stamp) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let fake = Fake { prepared: Arc::clone(&prepared), stamp: Arc::clone(&stamp), failures };
    let downloads = Downloads::new(folder.0.clone(), vec![Arc::new(fake)]);
    tokio::spawn(Room::new(own, downloads).serve(endpoint));
    Server { address, prepared, stamp, _folder: folder }
}

async fn start(server: &Server, name: &str) -> Result<Download, Error> {
    Download::start(&server.address, name.into(), |_, _| ()).await
}

#[tokio::test]
async fn a_download_is_prepared_once_and_its_files_come_whole_and_go_on_where_they_stopped() {
    let server = serve("whole", 0);
    let into = Folder::new("whole-into");
    let mut heard = Vec::new();
    let download = Download::start(&server.address, "first".into(), |done, total| heard.push((done, total))).await.unwrap();
    assert!(heard.iter().all(|&(done, total)| done <= total), "{heard:?}");
    let paths: Vec<_> = download.files().iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["small", "sub/large"]);

    let file = &download.files()[1];
    let to = into.0.join("large");
    download.fetch(file, &to, |_| ()).await.unwrap();
    // As if that fetch had stopped after the first kilobyte.
    std::fs::OpenOptions::new().write(true).open(&to).unwrap().set_len(1024).unwrap();
    let mut first = None;
    download.fetch(file, &to, |held| _ = first.get_or_insert(held)).await.unwrap();
    assert_eq!(first, Some(1024));
    assert_eq!(zstd::decode_all(std::fs::File::open(&to).unwrap()).unwrap(), large());

    // Asked for again, it is ready.
    download.close().await;
    start(&server, "first").await.unwrap().close().await;
    assert_eq!(server.prepared.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn a_file_that_came_damaged_is_removed_for_the_next_fetch_to_start_over() {
    let server = serve("damaged", 0);
    let into = Folder::new("damaged-into");
    let download = start(&server, "first").await.unwrap();
    let small = &download.files()[0];
    let to = into.0.join("small");
    // As long as the file, but not its beginning.
    std::fs::write(&to, vec![0; small.size as usize - 1]).unwrap();
    assert!(matches!(download.fetch(small, &to, |_| ()).await, Err(Error::Damaged(path)) if path == "small"));
    assert!(!to.exists());
    download.fetch(small, &to, |_| ()).await.unwrap();
    assert_eq!(zstd::decode_all(std::fs::File::open(&to).unwrap()).unwrap(), b"small");
}

#[tokio::test]
async fn a_download_that_failed_to_prepare_is_turned_away_and_prepared_anew_when_asked_again() {
    let server = serve("failed", 1);
    assert!(matches!(start(&server, "first").await, Err(Error::Rejected(Rejection::Unavailable))));
    start(&server, "first").await.unwrap().close().await;
    assert_eq!(server.prepared.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_download_not_offered_is_turned_away() {
    let server = serve("unoffered", 0);
    assert!(matches!(start(&server, "second").await, Err(Error::Rejected(Rejection::Unavailable))));
    assert_eq!(server.prepared.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_download_whose_source_changed_is_prepared_anew() {
    let server = serve("changed", 0);
    start(&server, "first").await.unwrap().close().await;
    server.stamp.store(1, Ordering::Relaxed);
    start(&server, "first").await.unwrap().close().await;
    start(&server, "first").await.unwrap().close().await;
    assert_eq!(server.prepared.load(Ordering::Relaxed), 2);
}
