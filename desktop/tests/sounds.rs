//! The server's sounds, installed by the app from a real server as they are there now.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use bukospeak_core::proto::Fingerprint;
use bukospeak_desktop::cues;
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_server::{Downloads, Identity, Room, Sounds};
use tokio::runtime::Runtime;

const ROOM_KEY: &str = "room-key";

/// A folder of the test's own, removed when it drops.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bukospeak-test-sounds-{name}-{}", std::process::id()));
        // Left over from a run that didn't finish, if at all.
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // A folder in the system's temporary folder left behind does no harm.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A server offering the sounds in its folder `sounds`.
struct Server {
    _runtime: Runtime,
    address: String,
    fingerprint: Fingerprint,
    sounds: PathBuf,
    _folder: Folder,
}

fn serve() -> Server {
    let runtime = Runtime::new().unwrap();
    let folder = Folder::new("server");
    let sounds = folder.0.join("sounds");
    fs::create_dir_all(&sounds).unwrap();
    let identity = Identity::generate().unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint(&identity, "127.0.0.1:0".parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    let downloads = Downloads::new(folder.0.join("downloads"), vec![Arc::new(Sounds::new(sounds.clone()))]);
    runtime.spawn(Room::new(ROOM_KEY.into(), downloads).serve(endpoint));
    Server { _runtime: runtime, address, fingerprint: identity.fingerprint(), sounds, _folder: folder }
}

#[test]
fn the_app_has_the_servers_sounds_as_they_are_there_now() {
    let server = serve();
    let app = Folder::new("app");
    let (known, installed) = (KnownServers::at(app.0.join("known_servers")), app.0.join("sounds"));
    let sync = || cues::sync(&server.address, ROOM_KEY.into(), &known, &installed).unwrap();
    fs::write(server.sounds.join("joined.wav"), b"first").unwrap();

    // A server not trusted yet isn't asked.
    sync();
    assert!(!installed.exists());

    known.insert(&server.address, server.fingerprint).unwrap();
    sync();
    assert_eq!(fs::read(installed.join("joined.wav")).unwrap(), b"first");

    fs::write(server.sounds.join("joined.wav"), b"second").unwrap();
    fs::write(server.sounds.join("left.wav"), b"third").unwrap();
    sync();
    assert_eq!(fs::read(installed.join("joined.wav")).unwrap(), b"second");
    assert_eq!(fs::read(installed.join("left.wav")).unwrap(), b"third");

    for sound in ["joined.wav", "left.wav"] {
        fs::remove_file(server.sounds.join(sound)).unwrap();
    }
    sync();
    assert!(!installed.exists());
}
