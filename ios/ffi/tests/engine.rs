use std::ffi::{CString, c_void};
use std::net::SocketAddr;
use std::ptr;
use std::slice;
use std::sync::mpsc;
use std::time::Duration;

use bukospeak_ffi::{
    BukoSpeakEngine, BukoSpeakError, BukoSpeakMember, BukoSpeakReason, BukoSpeakState, BukoSpeakStr, BukoSpeakUpdate,
    bukospeak_engine_free, bukospeak_engine_join, bukospeak_engine_leave, bukospeak_engine_new,
};
use bukospeak_server::{Device, Downloads, Room, Tailnet};
use tokio::runtime::Runtime;
#[derive(Debug, PartialEq)]
struct Heard {
    state: &'static str,
    you: u16,
    members: Vec<(u16, String)>,
    speaking: Vec<u16>,
    announcement: String,
    lost: Option<Failed>,
}
#[derive(Debug, PartialEq)]
struct Failed {
    reason: BukoSpeakReason,
    message: String,
}

impl Failed {
    fn of(error: &BukoSpeakError) -> Self {
        Self { reason: error.reason, message: text(&error.message) }
    }
}

fn text(text: &BukoSpeakStr) -> String {
    // SAFETY: Rust lends the text for the callback this runs in.
    String::from_utf8(unsafe { slice::from_raw_parts(text.ptr, text.len) }.to_vec()).unwrap()
}

fn members(members: *const BukoSpeakMember, len: usize) -> Vec<(u16, String)> {
    // SAFETY: Rust lends the members for the callback this runs in.
    unsafe { slice::from_raw_parts(members, len) }.iter().map(|member| (member.id, text(&member.name))).collect()
}

extern "C" fn heard(context: *mut c_void, update: &BukoSpeakUpdate) {
    // SAFETY: the context is the test's sender, which outlives the engine.
    let updates = unsafe { &*context.cast::<mpsc::Sender<Heard>>() };
    let state = match update.state {
        BukoSpeakState::Out => "out",
        BukoSpeakState::Connected => "connected",
        BukoSpeakState::Reconnecting => "reconnecting",
    };
    let heard = Heard {
        state,
        you: update.you,
        members: members(update.members, update.members_len),
        // SAFETY: lent for the callback.
        speaking: unsafe { slice::from_raw_parts(update.speaking, update.speaking_len) }.to_vec(),
        announcement: text(&update.announcement),
        // SAFETY: lent for the callback, if not null.
        lost: unsafe { update.lost.as_ref() }.map(Failed::of),
    };
    let _ = updates.send(heard);
}

extern "C" fn joined(context: *mut c_void, error: Option<&BukoSpeakError>) {
    // SAFETY: the context is the test's slot for the result, which outlives the join.
    let slot = unsafe { &mut *context.cast::<Option<Result<(), Failed>>>() };
    *slot = Some(error.map_or(Ok(()), |error| Err(Failed::of(error))));
}
fn device_per_connection(peer: SocketAddr) -> Option<Device> {
    Some(Device { id: peer.to_string(), name: format!("member {}", peer.port()) })
}
struct TailscaledDown;

impl Tailnet for TailscaledDown {
    fn whois(&self, _: SocketAddr) -> anyhow::Result<Option<Device>> {
        anyhow::bail!("tailscaled isn't running")
    }
}

struct Server {
    _runtime: Runtime,
    address: String,
}
fn serve(tailnet: impl Tailnet) -> Server {
    let runtime = Runtime::new().unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint("127.0.0.1:0".parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    runtime.spawn(Room::new(tailnet, Downloads::none()).serve(endpoint));
    Server { _runtime: runtime, address }
}
fn engine(updates: &mpsc::Sender<Heard>) -> Box<BukoSpeakEngine> {
    bukospeak_engine_new(ptr::from_ref(updates).cast_mut().cast(), heard).unwrap()
}

fn join(engine: &BukoSpeakEngine, server: &str) -> Result<(), Failed> {
    let server = CString::new(server).unwrap();
    let mut result = None;
    // SAFETY: the string is NUL-terminated, and the slot outlives the join.
    unsafe { bukospeak_engine_join(engine, server.as_ptr(), ptr::from_mut(&mut result).cast(), joined) };
    result.expect("the join hands its result over before it returns")
}

fn next(updates: &mpsc::Receiver<Heard>) -> Heard {
    updates.recv_timeout(Duration::from_secs(5)).expect("an update within 5 s")
}

#[test]
fn members_join_see_each_other_and_leave() {
    let server = serve(device_per_connection);
    let (first_updates, first_heard) = mpsc::channel();
    let (second_updates, second_heard) = mpsc::channel();
    let (first, second) = (engine(&first_updates), engine(&second_updates));

    join(&first, &server.address).unwrap();
    let alone = next(&first_heard);
    let first_id = alone.you;
    let [(_, first_name)] = &alone.members[..] else { panic!("the first member isn't alone: {alone:?}") };
    let first_name = first_name.clone();
    join(&second, &server.address).unwrap();
    let entered = next(&second_heard);
    let second_id = entered.you;
    let second_name = entered.members.iter().find(|(id, _)| *id == second_id).expect("the second member is in the channel").1.clone();
    let both = vec![(first_id, first_name.clone()), (second_id, second_name.clone())];
    let quiet = |state, you, members, announcement: &str| Heard {
        state,
        you,
        members,
        speaking: Vec::new(),
        announcement: announcement.into(),
        lost: None,
    };
    assert_eq!(entered, quiet("connected", second_id, both.clone(), ""));
    assert_eq!(next(&first_heard), quiet("connected", first_id, both, &format!("{second_name} ist beigetreten")));

    bukospeak_engine_leave(&second);
    assert_eq!(next(&second_heard), quiet("out", 0, Vec::new(), ""));
    assert_eq!(
        next(&first_heard),
        quiet("connected", first_id, vec![(first_id, first_name)], &format!("{second_name} hat den Kanal verlassen"))
    );
    bukospeak_engine_free(second);
    bukospeak_engine_free(first);
}

#[test]
fn a_device_the_tailnet_doesnt_know_is_told_it_is_a_stranger() {
    let server = serve(|_| None);
    let (updates, _heard) = mpsc::channel();
    let engine = engine(&updates);
    let failed = join(&engine, &server.address).expect_err("a stranger joined");
    assert_eq!(failed.reason, BukoSpeakReason::Stranger);
    assert!(!failed.message.is_empty());
    bukospeak_engine_free(engine);
}

#[test]
fn a_server_that_cant_ask_its_tailnet_leaves_the_device_unidentified() {
    let server = serve(TailscaledDown);
    let (updates, _heard) = mpsc::channel();
    let engine = engine(&updates);
    assert_eq!(join(&engine, &server.address).expect_err("an unidentified device joined").reason, BukoSpeakReason::Unidentified);
    bukospeak_engine_free(engine);
}

#[test]
fn a_call_whose_device_joined_elsewhere_is_lost_as_replaced() {
    let server = serve(|_| Some(Device { id: "phone".into(), name: "member".into() }));
    let (first_updates, first_heard) = mpsc::channel();
    let (second_updates, _second_heard) = mpsc::channel();
    let (first, second) = (engine(&first_updates), engine(&second_updates));
    join(&first, &server.address).unwrap();
    join(&second, &server.address).unwrap();
    let lost = loop {
        if let Some(lost) = next(&first_heard).lost {
            break lost;
        }
    };
    assert_eq!(lost.reason, BukoSpeakReason::Replaced);
    bukospeak_engine_free(second);
    bukospeak_engine_free(first);
}
