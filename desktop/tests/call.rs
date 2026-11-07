//! The call against a real server, as the app drives it, with stand-ins for the devices.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bukospeak_core::proto::MemberId;
use bukospeak_core::proto::control::Rejection;
use bukospeak_core::{Meter, Microphone, Speaker};
use bukospeak_desktop::audio::{DeviceId, Direction};
use bukospeak_desktop::call::{Call, Devices, Failure, Happening, Join, Phase, Player, Playing, Report};
use bukospeak_server::{Device, Downloads, Room, Tailnet};
use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use tokio::runtime::Runtime;

/// How long a call may take to get where a test waits for it: a lost connection is told after
/// a join again that fails.
const PATIENCE: Duration = Duration::from_secs(30);

/// A server, which stops when dropped as the real one does: it closes its connections, so
/// clients learn at once that it went.
struct Server {
    runtime: Runtime,
    endpoint: quinn::Endpoint,
    address: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"server stopped");
        self.runtime.block_on(self.endpoint.wait_idle());
    }
}

/// A tailnet in which each connection is a device of its own.
fn own(peer: SocketAddr) -> Option<Device> {
    Some(Device { id: peer.to_string(), name: format!("member {}", peer.port()) })
}

/// A tailnet in which every connection is the same device.
fn one_device(_: SocketAddr) -> Option<Device> {
    Some(Device { id: "desktop".into(), name: "Desktop Owner".into() })
}

/// A tailnet that knows no one.
fn nobody(_: SocketAddr) -> Option<Device> {
    None
}

/// A server on localhost whose members `tailnet` tells.
fn serve(tailnet: impl Tailnet) -> Server {
    let runtime = Runtime::new().unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint("127.0.0.1:0".parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    runtime.spawn(Room::new(tailnet, Downloads::none()).serve(endpoint.clone()));
    Server { runtime, endpoint, address }
}

/// Plays nothing, but holds the call's ends as the devices would, and counts the calls played
/// and the inputs listened to; or fails to start a call, as devices may.
#[derive(Clone, Default)]
struct StandIn {
    played: Arc<AtomicUsize>,
    playing: Arc<AtomicUsize>,
    listening: Arc<AtomicUsize>,
    fails: Arc<AtomicBool>,
}

/// A call's ends or the input held, counted in `count` while held.
struct Played {
    _ends: Option<(Microphone, Speaker)>,
    count: Arc<AtomicUsize>,
}

impl Played {
    fn held(ends: Option<(Microphone, Speaker)>, count: &Arc<AtomicUsize>) -> Box<dyn Playing> {
        count.fetch_add(1, Ordering::Relaxed);
        Box::new(Self { _ends: ends, count: Arc::clone(count) })
    }
}

impl Player for StandIn {
    fn play(&mut self, microphone: Microphone, speaker: Speaker, _: &Meter, _: &Devices, _: Report) -> std::io::Result<Box<dyn Playing>> {
        if self.fails.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("the stand-in fails"));
        }
        self.played.fetch_add(1, Ordering::Relaxed);
        Ok(Played::held(Some((microphone, speaker)), &self.playing))
    }

    fn listen(&mut self, _: &Meter, _: Option<&DeviceId>, _: Report) -> std::io::Result<Box<dyn Playing>> {
        Ok(Played::held(None, &self.listening))
    }
}

impl Playing for Played {
    fn set_device(&self, _: Direction, _: Option<DeviceId>) {}
}

impl Drop for Played {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A call as the app holds it.
struct App {
    call: Call,
    happenings: UnboundedReceiver<Happening>,
    devices: StandIn,
}

impl App {
    fn new() -> Self {
        let devices = StandIn::default();
        let (call, happenings) = Call::new(devices.clone()).unwrap();
        Self { call, happenings, devices }
    }

    fn join(&mut self, server: &Server) {
        self.call.join(Join { server: server.address.clone(), devices: Devices::default() });
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

    /// Who is in the call, in the order they came.
    fn ids(&self) -> Vec<MemberId> {
        self.call.members().iter().map(|member| member.id).collect()
    }

    /// This member's id, once in the call.
    fn you(&self) -> MemberId {
        self.call.you().expect("in the call")
    }
}

fn connected(call: &Call) -> bool {
    *call.phase() == Phase::Connected
}

#[test]
fn a_call_plays_while_it_is_up() {
    let server = serve(own);
    let mut app = App::new();
    app.join(&server);
    assert_eq!(*app.call.phase(), Phase::Joining);
    assert_eq!(app.call.since(), None);
    app.until("the call", connected);
    assert!(app.call.since().is_some());
    assert_eq!(app.ids(), [app.you()]);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);

    app.call.leave();
    assert_eq!(*app.call.phase(), Phase::Idle);
    assert!(app.call.members().is_empty());
    assert_eq!(app.call.since(), None);
    app.settle();
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 0);
}

#[test]
fn the_input_is_listened_to_while_asked_and_out_of_a_call_only() {
    let (server, stranger) = (serve(own), serve(nobody));
    let mut app = App::new();
    let listening = Arc::clone(&app.devices.listening);
    app.call.listen(true);
    app.settle();
    assert_eq!(listening.load(Ordering::Relaxed), 1);

    // The call's input takes over once the call plays, and gives it back when left.
    app.join(&server);
    app.until("the call", connected);
    assert_eq!(listening.load(Ordering::Relaxed), 0);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);
    app.call.leave();
    app.settle();
    assert_eq!((listening.load(Ordering::Relaxed), app.devices.playing.load(Ordering::Relaxed)), (1, 0));

    // A join that fails leaves it listened to.
    app.join(&stranger);
    app.until("the rejection", |call| matches!(call.phase(), Phase::Failed(_)));
    app.settle();
    assert_eq!(listening.load(Ordering::Relaxed), 1);

    app.call.listen(false);
    app.settle();
    assert_eq!(listening.load(Ordering::Relaxed), 0);
    app.join(&server);
    app.until("the call", connected);
    app.call.leave();
    app.settle();
    assert_eq!(listening.load(Ordering::Relaxed), 0, "not asked to, it isn't listened to after a call");
}

#[test]
fn the_lobby_sees_who_is_in_the_channel() {
    let server = serve(own);
    let mut lobby = App::new();
    lobby.call.watch(Some(server.address.clone()));

    let mut other = App::new();
    other.join(&server);
    other.until("the other's call", connected);
    let id = other.you();
    lobby.until("the other present", |call| call.present().iter().any(|member| member.id == id));
}

#[test]
fn a_leave_during_the_join_ends_the_call_before_it_plays_and_the_next_join_is_clean() {
    let server = serve(own);
    let mut other = App::new();
    other.join(&server);
    other.until("the other's call", connected);

    let mut app = App::new();
    app.join(&server);
    app.call.leave();
    assert_eq!(*app.call.phase(), Phase::Idle);
    // Joined again at once: this join waits for the one left, and sees nothing of it.
    app.join(&server);
    app.until("the second call", connected);
    assert_eq!(app.devices.played.load(Ordering::Relaxed), 1, "the call left during its join never played");
    other.until("the other seeing it", |call| call.members().len() == 2);
    app.settle();
    assert_eq!(app.ids(), [other.you(), app.you()]);
    assert_eq!(*app.call.phase(), Phase::Connected);

    app.call.leave();
    app.join(&server);
    app.until("the third call", connected);
    app.settle();
    assert_eq!(app.ids(), [other.you(), app.you()]);
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 1);
}

#[test]
fn quitting_during_a_join_leaves_the_channel() {
    let server = serve(own);
    let mut other = App::new();
    other.join(&server);
    other.until("the other's call", connected);

    let mut app = App::new();
    app.join(&server);
    // Waits for the join under way and leaves.
    drop(app);
    other.settle();
    assert_eq!(other.ids(), [other.you()]);
}

#[test]
fn audio_that_doesnt_start_fails_the_call_and_leaves() {
    let server = serve(own);
    let mut other = App::new();
    other.join(&server);
    other.until("the other's call", connected);

    let mut app = App::new();
    app.devices.fails.store(true, Ordering::Relaxed);
    app.join(&server);
    app.until("the failure", |call| *call.phase() == Phase::Failed(Failure::Audio));
    app.settle();
    assert!(app.call.members().is_empty());
    other.until("the other alone again", |call| call.members().len() == 1);

    app.devices.fails.store(false, Ordering::Relaxed);
    app.join(&server);
    app.until("the call", connected);
    assert_eq!(app.ids(), [other.you(), app.you()]);
}

#[test]
fn a_device_the_servers_tailnet_doesnt_know_is_told() {
    let server = serve(nobody);
    let mut app = App::new();
    app.join(&server);
    app.until("the rejection", |call| *call.phase() == Phase::Failed(Failure::Rejected(Rejection::Stranger)));
    app.call.dismiss();
    assert_eq!(*app.call.phase(), Phase::Idle);
}

#[test]
fn a_dropout_shows_reconnecting_and_a_lost_connection_ends_the_call_and_its_audio() {
    let server = serve(own);
    let mut app = App::new();
    app.join(&server);
    app.until("the call", connected);
    drop(server);
    // The call shows it, and plays on while it joins again, until that fails.
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
fn a_call_this_device_joined_again_elsewhere_ends_and_stays_ended() {
    let server = serve(one_device);
    let mut app = App::new();
    app.join(&server);
    app.until("the call", connected);

    let mut elsewhere = App::new();
    elsewhere.join(&server);
    elsewhere.until("the call elsewhere", connected);
    app.until("the replacement", |call| *call.phase() == Phase::Failed(Failure::Replaced));
    app.settle();
    assert_eq!(app.devices.playing.load(Ordering::Relaxed), 0);
    // The place stays elsewhere: the call ended here doesn't take it back.
    elsewhere.settle();
    assert_eq!(*elsewhere.call.phase(), Phase::Connected);
    assert_eq!(*app.call.phase(), Phase::Failed(Failure::Replaced));
}

#[test]
fn mute_holds_across_calls_and_the_others_see_it() {
    let server = serve(own);
    let mut other = App::new();
    other.join(&server);
    other.until("the other's call", connected);

    let mut app = App::new();
    app.call.set_muted(true);
    app.join(&server);
    app.until("the call", connected);
    let id = app.you();
    let muted = |muted: bool| move |call: &Call| call.members().iter().any(|member| member.id == id && member.muted == muted);
    other.until("it muted", muted(true));

    app.call.set_muted(false);
    other.until("it unmuted", muted(false));
    app.call.set_muted(true);
    app.call.leave();
    app.join(&server);
    app.until("the next call", connected);
    assert!(app.call.is_muted());
    let id = app.you();
    other.until("it muted again", move |call: &Call| call.members().iter().any(|member| member.id == id && member.muted));
}
