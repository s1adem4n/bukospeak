//! The call against a real server, as the app drives it, with stand-ins for the devices.

use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_core::proto::Fingerprint;
use bukospeak_core::proto::control::Rejection;
use bukospeak_core::{Microphone, Speaker};
use bukospeak_desktop::audio::{DeviceId, Direction};
use bukospeak_desktop::call::{Call, Devices, Failure, Happening, Join, Phase, Player, Playing, Report};
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_server::{Identity, Room};
use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use tokio::runtime::Runtime;

const ROOM_KEY: &str = "room-key";

/// How long a call may take to get where a test waits for it: a lost connection is told after
/// QUIC's idle timeout and a join again that fails.
const PATIENCE: Duration = Duration::from_secs(30);

struct Server {
    _runtime: Runtime,
    address: String,
    fingerprint: Fingerprint,
}

fn serve() -> Server {
    serve_at("127.0.0.1:0")
}

/// A server with a new identity at `address`.
fn serve_at(address: &str) -> Server {
    let runtime = Runtime::new().unwrap();
    let identity = Identity::generate().unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint(&identity, address.parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    runtime.spawn(Room::new(ROOM_KEY.into()).serve(endpoint));
    Server { _runtime: runtime, address, fingerprint: identity.fingerprint() }
}

/// Plays nothing, but holds the call's ends as the devices would, and counts the calls played;
/// or fails to start, as devices may.
#[derive(Clone, Default)]
struct StandIn {
    played: Arc<AtomicUsize>,
    playing: Arc<AtomicUsize>,
    fails: Arc<AtomicBool>,
}

struct Played {
    _ends: (Microphone, Speaker),
    playing: Arc<AtomicUsize>,
}

impl Player for StandIn {
    fn play(&mut self, microphone: Microphone, speaker: Speaker, _: &Devices, _: Report) -> std::io::Result<Box<dyn Playing>> {
        if self.fails.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("the stand-in fails"));
        }
        self.played.fetch_add(1, Ordering::Relaxed);
        self.playing.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Played { _ends: (microphone, speaker), playing: Arc::clone(&self.playing) }))
    }
}

impl Playing for Played {
    fn set_device(&self, _: Direction, _: Option<DeviceId>) {}
}

impl Drop for Played {
    fn drop(&mut self) {
        self.playing.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A call as the app holds it, with its known servers in a folder of the test's own.
struct App {
    call: Call,
    happenings: UnboundedReceiver<Happening>,
    devices: StandIn,
    known: std::path::PathBuf,
}

impl App {
    fn new(name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("bukospeak-call-{name}-{}", process::id()));
        let _ = std::fs::remove_dir_all(&folder); // Left over from an earlier run, if at all.
        let known = folder.join("known_servers");
        let devices = StandIn::default();
        let (call, happenings) = Call::new(KnownServers::at(known.clone()), devices.clone()).unwrap();
        Self { call, happenings, devices, known }
    }

    fn join(&mut self, server: &Server, name: &str, room_key: &str) {
        self.call.join(join(server, name, room_key));
    }

    /// Takes in what happens until `done` holds; fails the test after a while.
    fn until(&mut self, what: &str, done: impl Fn(&Call) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.call) {
            assert!(Instant::now() < deadline, "waited in vain for {what}: {:?}", self.call.phase());
            match self.happenings.try_recv() {
                Ok(happening) => self.call.happen(happening),
                Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(5)),
                Err(TryRecvError::Closed) => panic!("the call's happenings ended"),
            }
        }
    }

    /// Takes in what happens for a moment, as the app does all along.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match self.happenings.try_recv() {
                Ok(happening) => self.call.happen(happening),
                Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(5)),
                Err(TryRecvError::Closed) => panic!("the call's happenings ended"),
            }
        }
    }

    fn names(&self) -> Vec<&str> {
        self.call.members().iter().map(|member| member.name.as_str()).collect()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.known.parent().unwrap()); // It may never have been made.
    }
}

fn join(server: &Server, name: &str, room_key: &str) -> Join {
    Join { server: server.address.clone(), name: name.into(), room_key: room_key.into(), devices: Devices::default() }
}

fn connected(call: &Call) -> bool {
    *call.phase() == Phase::Connected
}

#[test]
fn a_call_plays_while_it_is_up_and_pins_the_server() {
    let server = serve();
    let mut app = App::new("plays");
    app.join(&server, "first", ROOM_KEY);
    assert_eq!(*app.call.phase(), Phase::Joining);
    assert_eq!(app.call.since(), None);
    app.until("the call", connected);
    assert!(app.call.since().is_some());
    assert_eq!(app.names(), ["first"]);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);
    assert_eq!(KnownServers::at(app.known.clone()).get(&server.address).unwrap(), Some(server.fingerprint));

    app.call.leave();
    assert_eq!(*app.call.phase(), Phase::Idle);
    assert!(app.call.members().is_empty());
    assert_eq!(app.call.since(), None);
    app.settle();
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 0);
}

#[test]
fn the_lobby_sees_who_is_in_the_channel() {
    let server = serve();
    let mut lobby = App::new("lobby");
    // Watching needs a server joined before.
    lobby.join(&server, "first", ROOM_KEY);
    lobby.until("the call", connected);
    lobby.call.leave();
    lobby.call.watch(Some((server.address.clone(), ROOM_KEY.into())));

    let mut other = App::new("lobby-other");
    other.join(&server, "second", ROOM_KEY);
    other.until("the other's call", connected);
    lobby.until("the other present", |call| call.present().iter().any(|member| member.name == "second"));
}

#[test]
fn a_leave_during_the_join_ends_the_call_before_it_plays_and_the_next_join_is_clean() {
    let server = serve();
    let mut other = App::new("clean-other");
    other.join(&server, "second", ROOM_KEY);
    other.until("the other's call", connected);

    let mut app = App::new("clean");
    app.join(&server, "first", ROOM_KEY);
    app.call.leave();
    assert_eq!(*app.call.phase(), Phase::Idle);
    // Joined again at once: this join waits for the one left, and sees nothing of it.
    app.join(&server, "first", ROOM_KEY);
    app.until("the second call", connected);
    assert_eq!(app.devices.played.load(Ordering::Relaxed), 1, "the call left during its join never played");
    other.until("the other seeing it", |call| call.members().len() == 2);
    app.settle();
    assert_eq!(app.names(), ["second", "first"]);
    assert_eq!(*app.call.phase(), Phase::Connected);

    app.call.leave();
    app.join(&server, "first", ROOM_KEY);
    app.until("the third call", connected);
    app.settle();
    assert_eq!(app.names(), ["second", "first"]);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);
}

#[test]
fn quitting_during_a_join_leaves_the_channel() {
    let server = serve();
    let mut other = App::new("quit-other");
    other.join(&server, "second", ROOM_KEY);
    other.until("the other's call", connected);

    let mut app = App::new("quit");
    app.join(&server, "first", ROOM_KEY);
    // Waits for the join under way and leaves.
    drop(app);
    other.settle();
    assert_eq!(other.names(), ["second"]);
}

#[test]
fn a_changed_certificate_is_trusted_only_as_presented() {
    let server = serve();
    let mut app = App::new("certificate");
    let pinned = Fingerprint([7; 32]);
    KnownServers::at(app.known.clone()).insert(&server.address, pinned).unwrap();
    app.join(&server, "first", ROOM_KEY);
    let changed = Failure::CertificateChanged { server: server.address.clone(), pinned, presented: server.fingerprint };
    app.until("the changed certificate", |call| *call.phase() == Phase::Failed(changed.clone()));
    assert_eq!(app.devices.played.load(Ordering::Relaxed), 0);

    app.call.trust(join(&server, "first", ROOM_KEY));
    app.until("the call", connected);
    assert_eq!(KnownServers::at(app.known.clone()).get(&server.address).unwrap(), Some(server.fingerprint));
}

#[test]
fn audio_that_doesnt_start_fails_the_call_and_leaves() {
    let server = serve();
    let mut other = App::new("audio-other");
    other.join(&server, "second", ROOM_KEY);
    other.until("the other's call", connected);

    let mut app = App::new("audio");
    app.devices.fails.store(true, Ordering::Relaxed);
    app.join(&server, "first", ROOM_KEY);
    app.until("the failure", |call| *call.phase() == Phase::Failed(Failure::Audio));
    app.settle();
    assert!(app.call.members().is_empty());
    other.until("the other alone again", |call| call.members().len() == 1);

    app.devices.fails.store(false, Ordering::Relaxed);
    app.join(&server, "first", ROOM_KEY);
    app.until("the call", connected);
    assert_eq!(app.names(), ["second", "first"]);
}

#[test]
fn a_wrong_room_key_is_told() {
    let server = serve();
    let mut app = App::new("room-key");
    app.join(&server, "first", "wrong");
    app.until("the rejection", |call| *call.phase() == Phase::Failed(Failure::Rejected(Rejection::RoomKey)));
    app.call.dismiss();
    assert_eq!(*app.call.phase(), Phase::Idle);
}

#[test]
fn a_dropout_shows_reconnecting_and_a_lost_connection_ends_the_call_and_its_audio() {
    let server = serve();
    let mut app = App::new("lost");
    app.join(&server, "first", ROOM_KEY);
    app.until("the call", connected);
    drop(server);
    // Nothing comes for a few seconds: the call shows it, and plays on, long before it is lost.
    let since = app.call.since();
    app.until("reconnecting", |call| *call.phase() == Phase::Reconnecting);
    // Joining again is still the same call.
    assert_eq!(app.call.since(), since);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);
    app.until("the loss", |call| *call.phase() == Phase::Failed(Failure::ConnectionLost));
    app.settle();
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 0);
}

#[test]
fn a_server_set_up_anew_during_the_call_asks_to_trust_its_certificate() {
    let server = serve();
    let mut app = App::new("anew");
    app.join(&server, "first", ROOM_KEY);
    app.until("the call", connected);
    let (address, pinned) = (server.address.clone(), server.fingerprint);
    drop(server);
    let server = serve_at(&address);
    let changed = Failure::CertificateChanged { server: address, pinned, presented: server.fingerprint };
    app.until("the changed certificate", |call| *call.phase() == Phase::Failed(changed.clone()));
    app.settle();
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 0);

    app.call.trust(join(&server, "first", ROOM_KEY));
    app.until("the call", connected);
}

#[test]
fn mute_holds_across_calls_and_the_others_see_it() {
    let server = serve();
    let mut other = App::new("mute-other");
    other.join(&server, "second", ROOM_KEY);
    other.until("the other's call", connected);

    let mut app = App::new("mute");
    app.call.set_muted(true);
    app.join(&server, "first", ROOM_KEY);
    app.until("the call", connected);
    let first_muted = |muted: bool| move |call: &Call| call.members().iter().any(|member| member.name == "first" && member.muted == muted);
    other.until("it muted", first_muted(true));

    app.call.set_muted(false);
    other.until("it unmuted", first_muted(false));
    app.call.set_muted(true);
    app.call.leave();
    app.join(&server, "first", ROOM_KEY);
    app.until("the next call", connected);
    assert!(app.call.is_muted());
    other.until("it muted again", first_muted(true));
}
