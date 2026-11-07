//! The C interface the iOS app calls the engine through: one engine per app, joining and leaving
//! the channel, and the call's audio on the voice processing unit.
//!
//! Off iOS the crate builds without the audio, so its tests run the interface against a real
//! server on the host.
//!
//! What Rust hands the app (updates, a join's result, why something failed) is lent for the
//! duration of the callback it comes with. Callbacks come on the engine's threads, or on the
//! caller's during a call that takes one. No pointer may be null unless its function says so.

#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
mod voice_unit;

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::str::Utf8Error;
use std::sync::{Arc, Mutex};

use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::{Conditioning, Engine, Error, Joined, Listener, Lost, State, Suppression, Update};

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

#[repr(C)]
pub struct BukoSpeakMember {
    pub id: u16,
    pub name: BukoSpeakStr,
    pub muted: bool,
    /// The member's connection was lost, and the server holds its place for it to come back.
    pub reconnecting: bool,
}

impl BukoSpeakMember {
    fn of(member: &Member) -> Self {
        Self { id: member.id.0, name: BukoSpeakStr::of(&member.name), muted: member.muted, reconnecting: member.reconnecting }
    }
}

/// Whether this member is in the channel.
#[repr(C)]
pub enum BukoSpeakState {
    Out,
    Connected,
    /// Nothing comes from the server, or the connection was lost and the engine joins again.
    Reconnecting,
}

/// The channel after a change, and what changed.
#[repr(C)]
pub struct BukoSpeakUpdate {
    /// Which of the engine's joins the call is, counting from 1; 0 before the first. It stays
    /// once the call is over, so that its last updates tell it too.
    pub call: u64,
    pub state: BukoSpeakState,
    /// This member's id; 0, which the server never hands out, while out of the channel.
    pub you: u16,
    /// Everyone in the channel, this member included, in the order they came; kept while
    /// joining again, empty when out.
    pub members: *const BukoSpeakMember,
    pub members_len: usize,
    /// Who speaks now, in id order, this member among them.
    pub speaking: *const u16,
    pub speaking_len: usize,
    /// What to tell the user aloud about the change, in German; empty if nothing.
    pub announcement: BukoSpeakStr,
    /// Why the call ended without a leave; null otherwise.
    pub lost: *const BukoSpeakError,
    /// Who is in the channel while the engine watches it from outside, in the order they came.
    pub present: *const BukoSpeakMember,
    pub present_len: usize,
}

/// Hears the channel after each change, in order, on a thread of the engine's.
pub type BukoSpeakListener = extern "C" fn(context: *mut c_void, update: &BukoSpeakUpdate);

/// Why a join failed or a call was lost, as far as the app tells it apart.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BukoSpeakReason {
    /// Anything else: the server didn't answer, the connection broke, and the like.
    Other,
    /// The server's tailnet doesn't know this device: it isn't on that tailnet, or Tailscale is
    /// off.
    Stranger,
    /// The server couldn't ask its tailnet who this device is.
    Unidentified,
    /// This device joined the channel again elsewhere, which took the call's place.
    Replaced,
}

impl BukoSpeakReason {
    fn of(error: &Error) -> Self {
        match error {
            Error::Rejected(Rejection::Stranger) => Self::Stranger,
            Error::Rejected(Rejection::Unidentified) => Self::Unidentified,
            Error::Replaced => Self::Replaced,
            _ => Self::Other,
        }
    }

    /// Why a call was lost: after its connection was, why joining again failed.
    fn lost(lost: &Lost) -> Self {
        match lost {
            Lost::Unrecovered { rejoin, .. } => Self::of(rejoin),
            Lost::Replaced => Self::Replaced,
        }
    }
}

/// Why a join failed or a call was lost.
#[repr(C)]
pub struct BukoSpeakError {
    pub reason: BukoSpeakReason,
    /// What happened, in English, for the log.
    pub message: BukoSpeakStr,
}

/// Hears how a join went: null if it joined, otherwise why not.
pub type BukoSpeakJoinDone = extern "C" fn(context: *mut c_void, error: Option<&BukoSpeakError>);

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
    fn update(&self, update: Update) {
        let Update { channel, announcement, lost } = update;
        let members: Vec<BukoSpeakMember> = channel.members().iter().map(BukoSpeakMember::of).collect();
        let speaking: Vec<u16> = channel.speaking().iter().map(|id| id.0).collect();
        let present: Vec<BukoSpeakMember> = channel.present().iter().map(BukoSpeakMember::of).collect();
        let announcement = announcement.map(|announcement| announcement.to_string()).unwrap_or_default();
        let lost = lost.map(|lost| (BukoSpeakReason::lost(&lost), lost.to_string()));
        let lost = lost.as_ref().map(|(reason, message)| BukoSpeakError { reason: *reason, message: BukoSpeakStr::of(message) });
        let state = match channel.state() {
            State::Out => BukoSpeakState::Out,
            State::Connected => BukoSpeakState::Connected,
            State::Reconnecting => BukoSpeakState::Reconnecting,
        };
        (self.listener)(
            self.context,
            &BukoSpeakUpdate {
                call: channel.call(),
                state,
                you: channel.you().map_or(0, |you| you.0),
                members: members.as_ptr(),
                members_len: members.len(),
                speaking: speaking.as_ptr(),
                speaking_len: speaking.len(),
                announcement: BukoSpeakStr::of(&announcement),
                lost: lost.as_ref().map_or(ptr::null(), ptr::from_ref),
                present: present.as_ptr(),
                present_len: present.len(),
            },
        );
    }
}

/// iOS's voice processing unit conditions the microphone's signal; elsewhere the engine does.
const CONDITIONING: Conditioning = if cfg!(target_os = "ios") { Conditioning::Platform } else { Conditioning::Engine };

/// A new engine, or null if its threads couldn't start. The listener hears its updates with
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

/// Stops the call's audio, leaves the call, and returns once the listener had its last update;
/// so not from the listener.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_free(engine: Box<BukoSpeakEngine>) {
    drop(engine);
}

/// Leaves the call before, if there is one, and joins the channel of `server`, as `host:port`,
/// as this device, which the server knows from its tailnet. Blocks until the server welcomes
/// this member or turns it away, 5 s at most, and hands `done` the result before it returns. Who
/// is in the call comes as the next update.
///
/// # Safety
///
/// `server` isn't null and is NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_engine_join(
    engine: &BukoSpeakEngine,
    server: *const c_char,
    context: *mut c_void,
    done: BukoSpeakJoinDone,
) {
    // SAFETY: the caller vouches for the string.
    let Ok(server) = (unsafe { text(server) }) else {
        let message = BukoSpeakStr::of("the server isn't UTF-8");
        return done(context, Some(&BukoSpeakError { reason: BukoSpeakReason::Other, message }));
    };
    match engine.join(&server) {
        Ok(()) => done(context, None),
        Err(error) => {
            let message = error.to_string();
            done(context, Some(&BukoSpeakError { reason: BukoSpeakReason::of(&error), message: BukoSpeakStr::of(&message) }));
        }
    }
}

/// Stops the call's audio and leaves the call, once the server knows; without one, nothing.
/// Returns the `call` the engine joined last: updates of it or of a call before that are still
/// on their way to the listener are of a call over now.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_leave(engine: &BukoSpeakEngine) -> u64 {
    engine.leave()
}

/// Watches who is in the channel of `server` without joining it, until `bukospeak_engine_unwatch`
/// or a join; the updates tell them as present. Returns at once; a watch that fails shows nobody
/// and starts again after a while.
///
/// # Safety
///
/// `server` isn't null and is NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_engine_watch(engine: &BukoSpeakEngine, server: *const c_char) {
    // SAFETY: the caller vouches for the string.
    let Ok(server) = (unsafe { text(server) }) else {
        // An app's own settings are UTF-8; text that isn't can't name a server to watch.
        return;
    };
    engine.engine.watch(server);
}

/// Stops watching who is in the channel, if the engine does.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_unwatch(engine: &BukoSpeakEngine) {
    engine.engine.unwatch();
}

/// Whether this member is muted: the microphone sends silence, so a talkspurt ends as in a
/// pause, and the others see it muted. It holds across calls until set again.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_muted(engine: &BukoSpeakEngine, muted: bool) {
    engine.engine.set_muted(muted);
}

/// Whether the microphone's noise is suppressed (the default), or sent as captured.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_noise_suppression(engine: &BukoSpeakEngine, on: bool) {
    let suppression = if on { Suppression::RNNoise } else { Suppression::Off };
    // Without a thread to make it on, a call goes on with the suppression before; the next takes
    // this one.
    let _ = engine.engine.set_suppression(suppression);
}

/// How loud the others play: 1.0 as they come, up to [`bukospeak_max_volume`]; the limiter
/// bends the peaks a louder mix would clip. It holds across calls until set again.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_volume(engine: &BukoSpeakEngine, volume: f32) {
    engine.engine.set_volume(volume);
}

/// The loudest volume the others play at: 2.0, 6 dB above their level.
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_max_volume() -> f32 {
    bukospeak_core::MAX_VOLUME
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
    fn join(&self, server: &str) -> Result<(), Error> {
        let _calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.stop_call_audio();
        let Joined { microphone, speaker } = self.engine.join(server)?;
        #[cfg(target_os = "ios")]
        {
            *self.audio.lock().expect("the audio lock is never poisoned") = Audio::Idle(microphone, speaker);
        }
        // Off iOS the call goes without audio: there it runs only in the tests.
        #[cfg(not(target_os = "ios"))]
        drop((microphone, speaker));
        Ok(())
    }

    fn leave(&self) -> u64 {
        let _calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.stop_call_audio();
        self.engine.leave()
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
