#[cfg_attr(not(target_os = "ios"), allow(dead_code, reason = "off iOS the engine runs only in tests, without audio to play into"))]
mod cues;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
mod output;
#[cfg(target_os = "ios")]
mod voice_unit;

use std::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::str::Utf8Error;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use std::{ptr, slice, thread};

use bukospeak_core::cues::Cue;
use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::{Conditioning, Engine, Error, Failure, Joined, Listener, State, Suppression, Update};

use crate::cues::Cues;
#[cfg(target_os = "ios")]
use crate::ios::Audio;
#[cfg(target_os = "ios")]
use crate::output::Output;

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
    Version,
    Stranger,
    Unidentified,
    Full,
    Unavailable,
    NotFound,
    NoAnswer,
    Unreachable,
    Audio,
    ConnectionLost,
    Replaced,
}

impl From<Failure> for BukoSpeakReason {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Rejected(Rejection::Version) => Self::Version,
            Failure::Rejected(Rejection::Stranger) => Self::Stranger,
            Failure::Rejected(Rejection::Unidentified) => Self::Unidentified,
            Failure::Rejected(Rejection::Full) => Self::Full,
            Failure::Rejected(Rejection::Unavailable) => Self::Unavailable,
            Failure::NotFound => Self::NotFound,
            Failure::NoAnswer => Self::NoAnswer,
            Failure::Unreachable => Self::Unreachable,
            Failure::Audio => Self::Audio,
            Failure::ConnectionLost => Self::ConnectionLost,
            Failure::Replaced => Self::Replaced,
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

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BukoSpeakCue {
    Joined,
    Left,
    Came,
    Went,
    Muted,
    Unmuted,
}

impl BukoSpeakCue {
    fn cue(self) -> Cue {
        match self {
            Self::Joined => Cue::Joined,
            Self::Left => Cue::Left,
            Self::Came => Cue::Came,
            Self::Went => Cue::Went,
            Self::Muted => Cue::Muted,
            Self::Unmuted => Cue::Unmuted,
        }
    }
}

// Lets the device, a Bluetooth one too, play out what the unit handed it before it stops.
const FAREWELL_TAIL: Duration = Duration::from_millis(250);

pub struct BukoSpeakEngine {
    // Fields drop in order: the audio stops before the engine leaves the call.
    #[cfg(target_os = "ios")]
    audio: Mutex<Audio>,
    engine: Engine,
    cues: Arc<Mutex<Cues>>,
    one_at_a_time: Mutex<()>,
}

struct AppListener {
    context: *mut c_void,
    listener: BukoSpeakListener,
    cues: Arc<Mutex<Cues>>,
}

// SAFETY: the app's listener and context are made to be called from the engine's threads.
unsafe impl Send for AppListener {}
// SAFETY: as for Send; the engine calls the listener from one thread at a time.
unsafe impl Sync for AppListener {}

impl Listener for AppListener {
    fn update(&self, update: Update) {
        let Update { channel, announcement, lost } = update;
        lock(&self.cues).observe(&channel);
        let members: Vec<BukoSpeakMember> = channel.members().iter().map(BukoSpeakMember::of).collect();
        let speaking: Vec<u16> = channel.speaking().iter().map(|id| id.0).collect();
        let present: Vec<BukoSpeakMember> = channel.present().iter().map(BukoSpeakMember::of).collect();
        let announcement = announcement.map(|announcement| announcement.to_string()).unwrap_or_default();
        let lost = lost.map(|lost| (BukoSpeakReason::from(Failure::from(&lost)), lost.to_string()));
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
    let cues = Arc::default();
    let engine = Engine::new(CONDITIONING, Arc::new(AppListener { context, listener, cues: Arc::clone(&cues) })).ok()?;
    Some(Box::new(BukoSpeakEngine {
        #[cfg(target_os = "ios")]
        audio: Mutex::new(Audio::None),
        engine,
        cues,
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
        return done(context, Some(&BukoSpeakError { reason: BukoSpeakReason::NotFound, message }));
    };
    match engine.join(&server) {
        Ok(()) => done(context, None),
        Err(error) => {
            let message = error.to_string();
            done(
                context,
                Some(&BukoSpeakError { reason: BukoSpeakReason::from(Failure::from(&error)), message: BukoSpeakStr::of(&message) }),
            );
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
    lock(&engine.cues).mute(muted);
}

/// # Safety
///
/// `samples` is null, for the cue's own tone, or points to `len` samples at 48 kHz, mono.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_engine_set_sound(engine: &BukoSpeakEngine, cue: BukoSpeakCue, samples: *const f32, len: usize) {
    // SAFETY: the caller vouches for the samples.
    let samples = (!samples.is_null()).then(|| unsafe { slice::from_raw_parts(samples, len) }.to_vec());
    lock(&engine.cues).set_sound(cue.cue(), samples);
}

#[unsafe(no_mangle)]
pub extern "C" fn bukospeak_cue_file(cue: BukoSpeakCue) -> BukoSpeakStr {
    BukoSpeakStr::of(cue.cue().file())
}

/// # Safety
///
/// `server` and `folder` aren't null and are NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bukospeak_sync_sounds(
    server: *const c_char,
    folder: *const c_char,
    context: *mut c_void,
    failed: BukoSpeakFailed,
) -> bool {
    // SAFETY: the caller vouches for the strings.
    let (Ok(server), Ok(folder)) = (unsafe { text(server) }, unsafe { text(folder) }) else {
        failed(context, BukoSpeakStr::of("the server or folder isn't UTF-8"));
        return false;
    };
    match bukospeak_core::cues::sync(&server, Path::new(&folder)) {
        Ok(()) => true,
        Err(error) => {
            failed(context, BukoSpeakStr::of(&error.to_string()));
            false
        }
    }
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
        // Before the join, whose first update tells the call was joined.
        let cues = lock(&self.cues).player();
        let Joined { microphone, speaker } = self.engine.join(server)?;
        #[cfg(target_os = "ios")]
        {
            *self.audio.lock().expect("the audio lock is never poisoned") = Audio::Idle(microphone, Output::new(speaker, cues));
        }
        // Off iOS the engine runs only in tests, without audio.
        #[cfg(not(target_os = "ios"))]
        drop((microphone, speaker, cues));
        Ok(())
    }

    fn leave(&self) -> u64 {
        let _one_at_a_time = self.one_at_a_time.lock().expect("the join and leave lock is never poisoned");
        let asked = Instant::now();
        // First, so the call falls silent under the farewell as it ends.
        let farewell = lock(&self.cues).leave();
        let left = self.engine.leave();
        if self.plays() && !farewell.is_zero() {
            thread::sleep((farewell + FAREWELL_TAIL).saturating_sub(asked.elapsed()));
        }
        self.stop_call_audio();
        left
    }

    fn plays(&self) -> bool {
        #[cfg(target_os = "ios")]
        return matches!(*self.audio.lock().expect("the audio lock is never poisoned"), Audio::Playing(_));
        #[cfg(not(target_os = "ios"))]
        false
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

fn lock(cues: &Mutex<Cues>) -> MutexGuard<'_, Cues> {
    cues.lock().expect("the cues lock is never poisoned")
}
