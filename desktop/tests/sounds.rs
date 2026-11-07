use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use bukospeak_desktop::cues;
use bukospeak_server::{Device, Downloads, Room, Sounds};
use tokio::runtime::Runtime;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bukospeak-test-sounds-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server {
    _runtime: Runtime,
    address: String,
    sounds: PathBuf,
    _folder: Folder,
}

fn own(peer: SocketAddr) -> Option<Device> {
    Some(Device { id: peer.to_string(), name: format!("member {}", peer.port()) })
}

fn serve() -> Server {
    let runtime = Runtime::new().unwrap();
    let folder = Folder::new("server");
    let sounds = folder.0.join("sounds");
    fs::create_dir_all(&sounds).unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint("127.0.0.1:0".parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    let downloads = Downloads::new(folder.0.join("downloads"), vec![Arc::new(Sounds::new(sounds.clone()))]);
    runtime.spawn(Room::new(own, downloads).serve(endpoint));
    Server { _runtime: runtime, address, sounds, _folder: folder }
}

#[test]
fn the_app_has_the_servers_sounds_as_they_are_there_now() {
    let server = serve();
    let app = Folder::new("app");
    let installed = app.0.join("sounds");
    let sync = || cues::sync(&server.address, &installed).unwrap();
    fs::write(server.sounds.join("joined.wav"), b"first").unwrap();

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
