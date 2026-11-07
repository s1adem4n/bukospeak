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
    pub reconnecting: bool,
}

impl BukoSpeakMember {
    fn of(member: &Member) -> Self {
        Self { id: member.id.0, name: BukoSpeakStr::of(&member.name), muted: member.muted, reconnecting: member.reconnecting }
    }
}

#[repr(C)]
pub enum BukoSpeakState {
    Out,
    Connected,
    Reconnecting,
}

#[repr(C)]
pub struct BukoSpeakUpdate {
    pub call: u64,
    pub state: BukoSpeakState,
    pub you: u16,
    pub members: *const BukoSpeakMember,
    pub members_len: usize,
    pub speaking: *const u16,
    pub speaking_len: usize,
    pub announcement: BukoSpeakStr,
    pub lost: *const BukoSpeakError,
    pub present: *const BukoSpeakMember,
    pub present_len: usize,
}

pub type BukoSpeakListener = extern "C" fn(context: *mut c_void, update: &BukoSpeakUpdate);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BukoSpeakReason {
    Other,
    Stranger,
    Unidentified,
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

    fn lost(lost: &Lost) -> Self {
        match lost {
            Lost::Unrecovered { rejoin, .. } => Self::of(rejoin),
            Lost::Replaced => Self::Replaced,
        }
    }
}

#[repr(C)]
pub struct BukoSpeakError {
    pub reason: BukoSpeakReason,
    pub message: BukoSpeakStr,
}

pub type BukoSpeakJoinDone = extern "C" fn(context: *mut c_void, error: Option<&BukoSpeakError>);

pub type BukoSpeakFailed = extern "C" fn(context: *mut c_void, message: BukoSpeakStr);

pub struct BukoSpeakEngine {
    // Fields drop in order: the audio stops before the engine leaves the call.
    #[cfg(target_os = "ios")]
    audio: Mutex<Audio>,
    engine: Engine,
    one_at_a_time: Mutex<()>,
}

struct AppListener {
    context: *mut c_void,
    listener: BukoSpeakListener,
}

// SAFETY: the app's listener and context are made to be called from the engine's threads.
unsafe impl Send for AppListener {}
// SAFETY: as for Send; the engine calls the listener from one thread at a time.
unsafe impl Sync for AppListener {}

impl Listener for AppListener {
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

const CONDITIONING: Conditioning = if cfg!(target_os = "ios") { Conditioning::Platform } else { Conditioning::Engine };

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_new(context: *mut c_void, listener: BukoSpeakListener) -> Option<Box<BukoSpeakEngine>> {
    let engine = Engine::new(CONDITIONING, Arc::new(AppListener { context, listener })).ok()?;
    Some(Box::new(BukoSpeakEngine {
        #[cfg(target_os = "ios")]
        audio: Mutex::new(Audio::None),
        engine,
        one_at_a_time: Mutex::new(()),
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_free(engine: Box<BukoSpeakEngine>) {
    drop(engine);
}

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

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_leave(engine: &BukoSpeakEngine) -> u64 {
    engine.leave()
}

/// # Safety
///
/// `server` isn't null and is NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_engine_watch(engine: &BukoSpeakEngine, server: *const c_char) {
    // SAFETY: the caller vouches for the string.
    let Ok(server) = (unsafe { text(server) }) else {
        // The app's settings are UTF-8; text that isn't names no server to watch.
        return;
    };
    engine.engine.watch(server);
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_unwatch(engine: &BukoSpeakEngine) {
    engine.engine.unwatch();
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_muted(engine: &BukoSpeakEngine, muted: bool) {
    engine.engine.set_muted(muted);
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_noise_suppression(engine: &BukoSpeakEngine, on: bool) {
    let suppression = if on { Suppression::RNNoise } else { Suppression::Off };
    // If its thread doesn't start, the call keeps the old suppression; the next call takes this one.
    let _ = engine.engine.set_suppression(suppression);
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_set_volume(engine: &BukoSpeakEngine, volume: f32) {
    engine.engine.set_volume(volume);
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_max_volume() -> f32 {
    bukospeak_core::MAX_VOLUME
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_start_audio(engine: &BukoSpeakEngine, context: *mut c_void, failed: BukoSpeakFailed) -> bool {
    // Bound first so the lock is released before `failed`, which may call the engine.
    let started = engine.audio.lock().expect("the audio lock is never poisoned").start();
    match started {
        Ok(()) => true,
        Err(error) => {
            failed(context, BukoSpeakStr::of(&error.to_string()));
            false
        }
    }
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_engine_stop_audio(engine: &BukoSpeakEngine) {
    engine.audio.lock().expect("the audio lock is never poisoned").stop();
}

impl BukoSpeakEngine {
    fn join(&self, server: &str) -> Result<(), Error> {
        let _one_at_a_time = self.one_at_a_time.lock().expect("the join and leave lock is never poisoned");
        self.stop_call_audio();
        let Joined { microphone, speaker } = self.engine.join(server)?;
        #[cfg(target_os = "ios")]
        {
            *self.audio.lock().expect("the audio lock is never poisoned") = Audio::Idle(microphone, speaker);
        }
        // Off iOS the engine runs only in tests, without audio.
        #[cfg(not(target_os = "ios"))]
        drop((microphone, speaker));
        Ok(())
    }

    fn leave(&self) -> u64 {
        let _one_at_a_time = self.one_at_a_time.lock().expect("the join and leave lock is never poisoned");
        self.stop_call_audio();
        self.engine.leave()
    }

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
