//! The C interface the apps call the engine through: one engine per app, joining and leaving
//! the channel, the same for every app; and per platform the call's audio, on iOS on the voice
//! processing unit.
//!
//! What Rust hands the app (events, a join's result, why something failed) is lent for the
//! duration of the callback it comes with. Callbacks come on the engine's threads, or on the
//! caller's during a call that takes one. No pointer may be null unless its function says so.

#[cfg(target_os = "ios")]
mod ios;

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::str::Utf8Error;
use std::sync::{Arc, Mutex};

use bukospeak_core::proto::Fingerprint;
use bukospeak_core::proto::control::Member;
use bukospeak_core::{Conditioning, Engine, Error, Event, JoinConfig, Joined, Listener};

#[cfg(target_os = "ios")]
use crate::ios::Audio;

/// UTF-8 text Rust lends for the duration of a callback; not NUL-terminated.
#[repr(C)]
pub struct BukoSpeakStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl BukoSpeakStr {
    fn of(text: &str) -> Self {
        Self { ptr: text.as_ptr(), len: text.len() }
    }
}

/// A server's certificate fingerprint, its SHA-256.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BukoSpeakFingerprint {
    pub bytes: [u8; 32],
}

#[repr(C)]
pub struct BukoSpeakMember {
    pub id: u16,
    pub name: BukoSpeakStr,
}

impl BukoSpeakMember {
    fn of(member: &Member) -> Self {
        Self { id: member.id.0, name: BukoSpeakStr::of(&member.name) }
    }
}

#[repr(C)]
pub enum BukoSpeakEventKind {
    /// A call started: the events after it are this call's, the ones before an earlier call's.
    Entered,
    Joined,
    Left,
    Speaking,
    Disconnected,
}

/// Something that happened in the call; only its kind's fields mean something.
#[repr(C)]
pub struct BukoSpeakEvent {
    pub kind: BukoSpeakEventKind,
    /// Entered: this member's id.
    pub you: u16,
    /// Entered: everyone in the channel, this member included.
    pub members: *const BukoSpeakMember,
    pub members_len: usize,
    /// Joined: who joined. Left: who left, by id only.
    pub member: BukoSpeakMember,
    /// Speaking: who speaks now, in id order, this member among them.
    pub speaking: *const u16,
    pub speaking_len: usize,
    /// Disconnected: why the call ended, for the user.
    pub reason: BukoSpeakStr,
}

impl BukoSpeakEvent {
    fn new(kind: BukoSpeakEventKind) -> Self {
        let nobody = BukoSpeakMember { id: 0, name: BukoSpeakStr::of("") };
        Self {
            kind,
            you: 0,
            members: ptr::null(),
            members_len: 0,
            member: nobody,
            speaking: ptr::null(),
            speaking_len: 0,
            reason: BukoSpeakStr::of(""),
        }
    }
}

/// Hears the engine's events, in order, on a thread of the engine's.
pub type BukoSpeakListener = extern "C" fn(context: *mut c_void, event: &BukoSpeakEvent);

#[repr(C)]
pub enum BukoSpeakJoinStatus {
    Joined,
    /// The server presented another certificate than the one pinned: it was set up anew, or
    /// someone pretends to be it.
    FingerprintChanged,
    Failed,
}

/// How a join went; only its status's fields mean something.
#[repr(C)]
pub struct BukoSpeakJoinResult {
    pub status: BukoSpeakJoinStatus,
    /// Joined: the server's, to pin for the next join. FingerprintChanged: the one presented.
    pub fingerprint: BukoSpeakFingerprint,
    /// FingerprintChanged: the one pinned.
    pub pinned: BukoSpeakFingerprint,
    /// FingerprintChanged and Failed: what happened, for the user.
    pub message: BukoSpeakStr,
}

impl BukoSpeakJoinResult {
    fn new(status: BukoSpeakJoinStatus, message: &str) -> Self {
        let unknown = BukoSpeakFingerprint { bytes: [0; 32] };
        Self { status, fingerprint: unknown, pinned: unknown, message: BukoSpeakStr::of(message) }
    }
}

/// Hears a join's result.
pub type BukoSpeakJoinDone = extern "C" fn(context: *mut c_void, result: &BukoSpeakJoinResult);

/// Hears why something failed, for the user.
pub type BukoSpeakFailed = extern "C" fn(context: *mut c_void, message: BukoSpeakStr);

/// The engine: joins and leaves the channel, one call at a time.
pub struct BukoSpeakEngine {
    // Fields drop in order: the audio stops before the engine leaves the call.
    #[cfg(target_os = "ios")]
    audio: Mutex<Audio>,
    engine: Engine,
    /// Held through a join and a leave, so they happen one after the other.
    calls: Mutex<()>,
}

/// The app's listener and its context.
struct Foreign {
    context: *mut c_void,
    listener: BukoSpeakListener,
}

// SAFETY: the header tells the app that its listener and context are used on the engine's
// threads.
unsafe impl Send for Foreign {}
// SAFETY: as for Send; the engine calls the listener from one thread at a time.
unsafe impl Sync for Foreign {}

impl Listener for Foreign {
    fn event(&self, event: Event) {
        let tell = |event: &BukoSpeakEvent| (self.listener)(self.context, event);
        match event {
            Event::Entered { you, members } => {
                let members: Vec<BukoSpeakMember> = members.iter().map(BukoSpeakMember::of).collect();
                tell(&BukoSpeakEvent {
                    you: you.0,
                    members: members.as_ptr(),
                    members_len: members.len(),
                    ..BukoSpeakEvent::new(BukoSpeakEventKind::Entered)
                });
            }
            Event::Joined(member) => {
                tell(&BukoSpeakEvent { member: BukoSpeakMember::of(&member), ..BukoSpeakEvent::new(BukoSpeakEventKind::Joined) })
            }
            Event::Left(id) => {
                let mut event = BukoSpeakEvent::new(BukoSpeakEventKind::Left);
                event.member.id = id.0;
                tell(&event);
            }
            Event::Speaking(members) => {
                let ids: Vec<u16> = members.iter().map(|id| id.0).collect();
                tell(&BukoSpeakEvent {
                    speaking: ids.as_ptr(),
                    speaking_len: ids.len(),
                    ..BukoSpeakEvent::new(BukoSpeakEventKind::Speaking)
                });
            }
            Event::Disconnected(error) => {
                let reason = error.to_string();
                tell(&BukoSpeakEvent { reason: BukoSpeakStr::of(&reason), ..BukoSpeakEvent::new(BukoSpeakEventKind::Disconnected) });
            }
        }
    }
}

/// iOS's voice processing unit conditions the microphone's signal; elsewhere the engine does.
const CONDITIONING: Conditioning = if cfg!(target_os = "ios") { Conditioning::Platform } else { Conditioning::Engine };

/// A new engine, or null if its threads couldn't start. The listener hears its events with
/// `context` until `bukospeak_engine_free` returns, never after.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_new(context: *mut c_void, listener: BukoSpeakListener) -> Option<Box<BukoSpeakEngine>> {
    let engine = Engine::new(CONDITIONING, Arc::new(Foreign { context, listener })).ok()?;
    Some(Box::new(BukoSpeakEngine {
        #[cfg(target_os = "ios")]
        audio: Mutex::new(Audio::None),
        engine,
        calls: Mutex::new(()),
    }))
}

/// Stops the call's audio, leaves the call, and returns once the listener had its last event;
/// so not from the listener.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_free(engine: Box<BukoSpeakEngine>) {
    drop(engine);
}

/// Leaves the call before, if there is one, and joins the channel, trusting the server by its
/// fingerprint `pin`, or on first use if it is null. Blocks until the server welcomes this
/// member or turns it away, 5 s at most, and hands `done` the result before it returns. Who is
/// in the call comes as its Entered event.
///
/// # Safety
///
/// `server`, `name` and `room_key` aren't null and are NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_engine_join(
    engine: &BukoSpeakEngine,
    server: *const c_char,
    name: *const c_char,
    room_key: *const c_char,
    pin: Option<&BukoSpeakFingerprint>,
    context: *mut c_void,
    done: BukoSpeakJoinDone,
) {
    // SAFETY: the caller vouches for the strings.
    let texts = unsafe { (text(server), text(name), text(room_key)) };
    let (Ok(server), Ok(name), Ok(room_key)) = texts else {
        return done(context, &BukoSpeakJoinResult::new(BukoSpeakJoinStatus::Failed, "the server, name or room key isn't UTF-8"));
    };
    let pin = pin.map(|pin| Fingerprint(pin.bytes));
    match engine.join(JoinConfig { server, name, room_key, pin }) {
        Ok(fingerprint) => done(
            context,
            &BukoSpeakJoinResult {
                fingerprint: BukoSpeakFingerprint { bytes: fingerprint.0 },
                ..BukoSpeakJoinResult::new(BukoSpeakJoinStatus::Joined, "")
            },
        ),
        Err(error @ Error::FingerprintChanged { pinned, presented }) => {
            let message = error.to_string();
            done(
                context,
                &BukoSpeakJoinResult {
                    fingerprint: BukoSpeakFingerprint { bytes: presented.0 },
                    pinned: BukoSpeakFingerprint { bytes: pinned.0 },
                    ..BukoSpeakJoinResult::new(BukoSpeakJoinStatus::FingerprintChanged, &message)
                },
            );
        }
        Err(error) => done(context, &BukoSpeakJoinResult::new(BukoSpeakJoinStatus::Failed, &error.to_string())),
    }
}

/// Stops the call's audio and leaves the call, once the server knows; without one, nothing.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_leave(engine: &BukoSpeakEngine) {
    engine.leave();
}

/// Whether the microphone sends silence, so a talkspurt ends as in a pause; it holds across
/// calls until set again.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_muted(engine: &BukoSpeakEngine, muted: bool) {
    engine.engine.set_muted(muted);
}

/// Whether the microphone's noise is suppressed (the default), or sent as captured.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_noise_suppression(engine: &BukoSpeakEngine, on: bool) {
    engine.engine.set_noise_suppression(on);
}

/// Plays the call on the voice processing unit, once the app's audio session is active. True
/// if it plays, or there is no call; false if the unit didn't start, after `failed` heard why.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_start_audio(engine: &BukoSpeakEngine, context: *mut c_void, failed: BukoSpeakFailed) -> bool {
    // Started apart, so `failed` runs without the lock and may call the engine.
    let started = engine.audio.lock().expect("the audio lock is never poisoned").start();
    match started {
        Ok(()) => true,
        Err(error) => {
            failed(context, BukoSpeakStr::of(&error.to_string()));
            false
        }
    }
}

/// Stops playing the call, for an audio session that is no longer active: a talkspurt ends,
/// and nobody shows as speaking until it plays again.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_stop_audio(engine: &BukoSpeakEngine) {
    engine.audio.lock().expect("the audio lock is never poisoned").stop();
}

impl BukoSpeakEngine {
    fn join(&self, config: JoinConfig) -> Result<Fingerprint, Error> {
        let _calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.stop_call_audio();
        let Joined { fingerprint, microphone, speaker } = self.engine.join(config)?;
        #[cfg(target_os = "ios")]
        {
            *self.audio.lock().expect("the audio lock is never poisoned") = Audio::Idle(microphone, speaker);
        }
        // Only iOS plays a call through this interface so far; elsewhere it goes without audio.
        #[cfg(not(target_os = "ios"))]
        drop((microphone, speaker));
        Ok(fingerprint)
    }

    fn leave(&self) {
        let _calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.stop_call_audio();
        self.engine.leave();
    }

    /// Drops the call's halves, the devices stopped first, so nothing plays into a call that
    /// is gone.
    fn stop_call_audio(&self) {
        #[cfg(target_os = "ios")]
        {
            *self.audio.lock().expect("the audio lock is never poisoned") = Audio::None;
        }
    }
}

/// # Safety
///
/// `text` isn't null and is NUL-terminated.
unsafe fn text(text: *const c_char) -> Result<String, Utf8Error> {
    // SAFETY: the caller vouches for it.
    unsafe { CStr::from_ptr(text) }.to_str().map(str::to_owned)
}
