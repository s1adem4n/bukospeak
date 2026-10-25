//! The C interface against a real server, called as an app calls it.

use std::ffi::{CString, c_void};
use std::ptr;
use std::slice;
use std::sync::mpsc;
use std::time::Duration;

use bukospeak_ffi::{
    BukoSpeakEngine, BukoSpeakFingerprint, BukoSpeakJoinResult, BukoSpeakJoinStatus, BukoSpeakMember, BukoSpeakState, BukoSpeakStr,
    BukoSpeakUpdate, bukospeak_engine_free, bukospeak_engine_join, bukospeak_engine_leave, bukospeak_engine_new,
};
use bukospeak_server::{Identity, Room};
use tokio::runtime::Runtime;

const ROOM_KEY: &str = "room-key";

/// An update as the test keeps it, past the callback.
#[derive(Debug, PartialEq)]
struct Heard {
    /// The state's name.
    state: &'static str,
    you: u16,
    members: Vec<(u16, String)>,
    speaking: Vec<u16>,
    announcement: String,
    lost: String,
}

/// A join's result as the test keeps it.
#[derive(Debug, PartialEq)]
enum Outcome {
    Joined { fingerprint: [u8; 32] },
    FingerprintChanged { pinned: [u8; 32], presented: [u8; 32], message: String },
    Failed(String),
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
        lost: text(&update.lost),
    };
    // The test may have stopped listening.
    let _ = updates.send(heard);
}

extern "C" fn joined(context: *mut c_void, result: &BukoSpeakJoinResult) {
    // SAFETY: the context is the test's slot for the result, which outlives the join.
    let slot = unsafe { &mut *context.cast::<Option<Outcome>>() };
    *slot = Some(match result.status {
        BukoSpeakJoinStatus::Joined => Outcome::Joined { fingerprint: result.fingerprint.bytes },
        BukoSpeakJoinStatus::FingerprintChanged => {
            Outcome::FingerprintChanged { pinned: result.pinned.bytes, presented: result.fingerprint.bytes, message: text(&result.message) }
        }
        BukoSpeakJoinStatus::Failed => Outcome::Failed(text(&result.message)),
    });
}

struct Server {
    _runtime: Runtime,
    address: String,
    fingerprint: [u8; 32],
}

fn serve() -> Server {
    let runtime = Runtime::new().unwrap();
    let identity = Identity::generate().unwrap();
    let endpoint = runtime.block_on(async { bukospeak_server::endpoint(&identity, "127.0.0.1:0".parse().unwrap()) }).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    runtime.spawn(Room::new(ROOM_KEY.into()).serve(endpoint));
    Server { _runtime: runtime, address, fingerprint: identity.fingerprint().0 }
}

/// An engine whose updates go to the receiver; the sender must outlive it.
fn engine(updates: &mpsc::Sender<Heard>) -> Box<BukoSpeakEngine> {
    bukospeak_engine_new(ptr::from_ref(updates).cast_mut().cast(), heard).unwrap()
}

fn join(engine: &BukoSpeakEngine, server: &str, name: &str, room_key: &str, pin: Option<[u8; 32]>) -> Outcome {
    let (server, name, room_key) = (CString::new(server).unwrap(), CString::new(name).unwrap(), CString::new(room_key).unwrap());
    let pin = pin.map(|bytes| BukoSpeakFingerprint { bytes });
    let mut result = None;
    // SAFETY: the strings are NUL-terminated, and the slot outlives the join.
    unsafe {
        bukospeak_engine_join(
            engine,
            server.as_ptr(),
            name.as_ptr(),
            room_key.as_ptr(),
            pin.as_ref(),
            ptr::from_mut(&mut result).cast(),
            joined,
        );
    }
    result.expect("the join hands its result over before it returns")
}

fn next(updates: &mpsc::Receiver<Heard>) -> Heard {
    updates.recv_timeout(Duration::from_secs(5)).expect("an update within 5 s")
}

#[test]
fn members_join_see_each_other_and_leave() {
    let server = serve();
    let (first_events, first_heard) = mpsc::channel();
    let (second_events, second_heard) = mpsc::channel();
    let (first, second) = (engine(&first_events), engine(&second_events));

    let Outcome::Joined { fingerprint } = join(&first, &server.address, "first", ROOM_KEY, None) else {
        panic!("the first member didn't join")
    };
    assert_eq!(fingerprint, server.fingerprint, "a first join learns the fingerprint to pin");
    let first_id = next(&first_heard).you;
    assert!(matches!(join(&second, &server.address, "second", ROOM_KEY, Some(fingerprint)), Outcome::Joined { .. }));
    let entered = next(&second_heard);
    let second_id = entered.you;
    let both = vec![(first_id, "first".to_owned()), (second_id, "second".to_owned())];
    let quiet = |state, you, members, announcement: &str| Heard {
        state,
        you,
        members,
        speaking: Vec::new(),
        announcement: announcement.into(),
        lost: String::new(),
    };
    assert_eq!(entered, quiet("connected", second_id, both.clone(), ""));
    assert_eq!(next(&first_heard), quiet("connected", first_id, both, "second ist beigetreten"));

    bukospeak_engine_leave(&second);
    assert_eq!(next(&second_heard), quiet("out", 0, Vec::new(), ""));
    assert_eq!(next(&first_heard), quiet("connected", first_id, vec![(first_id, "first".to_owned())], "second hat den Kanal verlassen"));
    bukospeak_engine_free(second);
    bukospeak_engine_free(first);
}

#[test]
fn a_wrong_room_key_fails_with_a_message() {
    let server = serve();
    let (events, _heard) = mpsc::channel();
    let engine = engine(&events);
    let Outcome::Failed(message) = join(&engine, &server.address, "stranger", "guess", Some(server.fingerprint)) else {
        panic!("a wrong room key joined")
    };
    assert!(!message.is_empty());
    bukospeak_engine_free(engine);
}

#[test]
fn a_changed_certificate_is_told_with_both_fingerprints() {
    let server = serve();
    let (events, _heard) = mpsc::channel();
    let engine = engine(&events);
    let pinned = [7; 32];
    let result = join(&engine, &server.address, "first", ROOM_KEY, Some(pinned));
    let Outcome::FingerprintChanged { pinned: told, presented, message } = result else { panic!("joined with a wrong pin: {result:?}") };
    assert_eq!((told, presented), (pinned, server.fingerprint));
    assert!(!message.is_empty());
    bukospeak_engine_free(engine);
}
