//! A call on the iPhone's microphone and speaker, through the VoiceProcessingIO unit, which
//! cancels the speaker's echo in what the microphone hears and evens out its level.
//!
//! The app's audio session picks the route; the unit converts between it and the call's 48 kHz
//! mono, so a route change, to Bluetooth say, needs nothing here. A [`VoiceUnit`] plays while
//! the session is active and is stopped when it isn't.

#![cfg(target_os = "ios")]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::panic::{self, AssertUnwindSafe};
use std::ptr::{self, NonNull};
use std::time::{Duration, Instant};
use std::{process, slice};

use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::{Microphone, Speaker};
use objc2_audio_toolbox::{
    AURenderCallbackStruct, AUVoiceIOOtherAudioDuckingConfiguration, AUVoiceIOOtherAudioDuckingLevel, AudioComponentDescription,
    AudioComponentFindNext, AudioComponentInstanceDispose, AudioComponentInstanceNew, AudioOutputUnitStart, AudioOutputUnitStop, AudioUnit,
    AudioUnitInitialize, AudioUnitPropertyID, AudioUnitRender, AudioUnitRenderActionFlags, AudioUnitScope, AudioUnitSetProperty,
    AudioUnitUninitialize, kAUVoiceIOProperty_OtherAudioDuckingConfiguration, kAudioOutputUnitProperty_EnableIO,
    kAudioOutputUnitProperty_SetInputCallback, kAudioUnitErr_TooManyFramesToProcess, kAudioUnitManufacturer_Apple,
    kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitProperty_SetRenderCallback, kAudioUnitProperty_ShouldAllocateBuffer,
    kAudioUnitProperty_StreamFormat, kAudioUnitScope_Global, kAudioUnitScope_Input, kAudioUnitScope_Output,
    kAudioUnitSubType_VoiceProcessingIO, kAudioUnitType_Output,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, AudioTimeStampFlags, kAudio_ParamError,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked, kAudioFormatLinearPCM,
};

/// The most frames the unit asks for at once: iOS's slice while the screen is locked.
const MAX_FRAMES: u32 = 4096;

/// The unit's element that plays to the speaker, and the one that records the microphone.
const OUTPUT_BUS: u32 = 0;
const INPUT_BUS: u32 = 1;

const SAMPLE_BYTES: u32 = size_of::<f32>() as u32;

/// How many nanoseconds `numer / denom` host time ticks are.
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

// SAFETY: libSystem's host time, as declared in <mach/mach_time.h>; the host time in an
// `AudioTimeStamp` counts the same ticks. Reading it has no preconditions.
unsafe extern "C" {
    safe fn mach_absolute_time() -> u64;
    fn mach_timebase_info(timebase: *mut Timebase) -> i32;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this device has no voice processing unit")]
    NoUnit,
    #[error("{step} failed with OSStatus {status}")]
    Failed { step: &'static str, status: i32 },
}

/// A unit that didn't start, and the halves it was to play.
pub struct Failure {
    pub error: Error,
    pub microphone: Microphone,
    pub speaker: Speaker,
}

/// A call's halves playing on the VoiceProcessingIO unit, until it is stopped or dropped.
pub struct VoiceUnit {
    /// Null until the unit is created.
    unit: AudioUnit,
    // Owned through raw pointers, from `Box::into_raw`: the unit's callbacks use them while it
    // runs, on its thread.
    input: NonNull<Input>,
    speaker: NonNull<Speaker>,
}

// SAFETY: the halves are Send, and an AudioUnit may be stopped and disposed from any thread.
unsafe impl Send for VoiceUnit {}

/// What the input callback needs: the unit to fetch the microphone's samples from, room for
/// them, and the half they go to.
struct Input {
    unit: AudioUnit,
    samples: Box<[f32; MAX_FRAMES as usize]>,
    microphone: Microphone,
}

impl VoiceUnit {
    /// Creates the unit and starts it playing the halves, which come back if it can't.
    pub fn start(microphone: Microphone, speaker: Speaker) -> Result<Self, Box<Failure>> {
        let input = Box::new(Input { unit: ptr::null_mut(), samples: Box::new([0.0; MAX_FRAMES as usize]), microphone });
        let mut this =
            Self { unit: ptr::null_mut(), input: NonNull::from(Box::leak(input)), speaker: NonNull::from(Box::leak(Box::new(speaker))) };
        match this.set_up() {
            Ok(()) => Ok(this),
            Err(error) => {
                let (microphone, speaker) = this.stop();
                Err(Box::new(Failure { error, microphone, speaker }))
            }
        }
    }

    /// Stops and disposes the unit and hands the halves back, told they stopped.
    pub fn stop(self) -> (Microphone, Speaker) {
        let mut this = ManuallyDrop::new(self);
        // SAFETY: `this` is never used again, nor dropped.
        unsafe { this.release() }
    }

    fn set_up(&mut self) -> Result<(), Error> {
        let description = AudioComponentDescription {
            componentType: kAudioUnitType_Output,
            componentSubType: kAudioUnitSubType_VoiceProcessingIO,
            componentManufacturer: kAudioUnitManufacturer_Apple,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        // SAFETY: the description lives through the call.
        let component = unsafe { AudioComponentFindNext(ptr::null_mut(), NonNull::from(&description)) };
        if component.is_null() {
            return Err(Error::NoUnit);
        }
        // SAFETY: the component was just found, and the unit is written to a field of ours.
        check("creating the unit", unsafe { AudioComponentInstanceNew(component, NonNull::from(&mut self.unit)) })?;
        // SAFETY: nothing calls back yet, so nothing else uses the input.
        unsafe { self.input.as_mut() }.unit = self.unit;

        let format = AudioStreamBasicDescription {
            mSampleRate: f64::from(SAMPLE_RATE),
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: SAMPLE_BYTES,
            mFramesPerPacket: 1,
            mBytesPerFrame: SAMPLE_BYTES,
            mChannelsPerFrame: 1,
            mBitsPerChannel: 8 * SAMPLE_BYTES,
            mReserved: 0,
        };
        let capture = AURenderCallbackStruct { inputProc: Some(capture), inputProcRefCon: self.input.as_ptr().cast() };
        let render = AURenderCallbackStruct { inputProc: Some(render), inputProcRefCon: self.speaker.as_ptr().cast() };
        let ducking =
            AUVoiceIOOtherAudioDuckingConfiguration { mEnableAdvancedDucking: 0, mDuckingLevel: AUVoiceIOOtherAudioDuckingLevel::Min };
        // SAFETY: each value has the type its property takes. The callbacks' refcons are ours
        // and outlive the unit, which `release` disposes before it frees them.
        unsafe {
            self.set("enabling the microphone", kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, INPUT_BUS, &1u32)?;
            self.set("setting the microphone's format", kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, INPUT_BUS, &format)?;
            self.set("setting the speaker's format", kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, OUTPUT_BUS, &format)?;
            self.set("setting the largest slice", kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &MAX_FRAMES)?;
            // The input callback renders the microphone into room of its own.
            self.set(
                "leaving the microphone's buffer to us",
                kAudioUnitProperty_ShouldAllocateBuffer,
                kAudioUnitScope_Output,
                INPUT_BUS,
                &0u32,
            )?;
            self.set("setting the input callback", kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global, INPUT_BUS, &capture)?;
            self.set("setting the render callback", kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, OUTPUT_BUS, &render)?;
            // A game or music playing beside the call keeps its loudness.
            self.set("turning ducking down", kAUVoiceIOProperty_OtherAudioDuckingConfiguration, kAudioUnitScope_Global, 0, &ducking)?;
            check("initializing the unit", AudioUnitInitialize(self.unit))?;
            check("starting the unit", AudioOutputUnitStart(self.unit))
        }
    }

    /// Sets one of the unit's properties.
    ///
    /// # Safety
    ///
    /// `T` is the type the property takes, and pointers in it stay valid as the property needs.
    unsafe fn set<T>(
        &self,
        step: &'static str,
        property: AudioUnitPropertyID,
        scope: AudioUnitScope,
        element: u32,
        value: &T,
    ) -> Result<(), Error> {
        // SAFETY: the unit was created, and the caller vouches for the value.
        check(step, unsafe {
            AudioUnitSetProperty(self.unit, property, scope, element, ptr::from_ref(value).cast(), size_of::<T>() as u32)
        })
    }

    /// Disposes the unit, so its callbacks are done, and hands the halves back, told they
    /// stopped.
    ///
    /// # Safety
    ///
    /// Called once: the halves are freed, and `self` must not be used or dropped after.
    unsafe fn release(&mut self) -> (Microphone, Speaker) {
        if !self.unit.is_null() {
            // SAFETY: the unit was created and isn't used after. Stopping and uninitializing
            // are steps toward disposing, which alone decides whether the unit is gone; their
            // failure leaves it to dispose.
            let disposed = unsafe {
                let _ = AudioOutputUnitStop(self.unit);
                let _ = AudioUnitUninitialize(self.unit);
                AudioComponentInstanceDispose(self.unit)
            };
            // A unit that is still there may still call back into the halves, so freeing them
            // would leave it to use freed memory.
            if disposed != 0 {
                eprintln!("the voice unit couldn't be disposed: {disposed}");
                process::abort();
            }
        }
        // SAFETY: both came from `Box::leak` in `start`, and the unit that used them was
        // disposed above, so it calls neither callback again.
        let (input, speaker) = unsafe { (Box::from_raw(self.input.as_ptr()), Box::from_raw(self.speaker.as_ptr())) };
        let (mut microphone, mut speaker) = (input.microphone, *speaker);
        microphone.stopped();
        speaker.stopped();
        (microphone, speaker)
    }
}

impl Drop for VoiceUnit {
    fn drop(&mut self) {
        // SAFETY: `self` is gone after this.
        drop(unsafe { self.release() });
    }
}

/// The input callback: renders what the microphone heard and pushes it to the call.
unsafe extern "C-unwind" fn capture(
    refcon: NonNull<c_void>,
    flags: NonNull<AudioUnitRenderActionFlags>,
    timestamp: NonNull<AudioTimeStamp>,
    bus: u32,
    frames: u32,
    _: *mut AudioBufferList,
) -> i32 {
    abort_on_panic(|| {
        // SAFETY: the refcon is the unit's input, which outlives it, and only this callback
        // uses it while the unit runs.
        let input = unsafe { refcon.cast::<Input>().as_mut() };
        let Some(samples) = input.samples.get_mut(..frames as usize) else {
            return kAudioUnitErr_TooManyFramesToProcess;
        };
        let mut list = AudioBufferList {
            mNumberBuffers: 1,
            mBuffers: [AudioBuffer { mNumberChannels: 1, mDataByteSize: frames * SAMPLE_BYTES, mData: samples.as_mut_ptr().cast() }],
        };
        // SAFETY: the unit is live while it calls back, and the list has room for `frames`
        // samples in the format set.
        let status = unsafe { AudioUnitRender(input.unit, flags.as_ptr(), timestamp, bus, frames, NonNull::from(&mut list)) };
        if status == 0 {
            // SAFETY: the unit hands a time stamp that is valid during the callback.
            input.microphone.push(samples, instant(unsafe { timestamp.as_ref() }));
        }
        status
    })
}

/// The render callback: fills the speaker's buffer with the call's mix.
unsafe extern "C-unwind" fn render(
    refcon: NonNull<c_void>,
    _: NonNull<AudioUnitRenderActionFlags>,
    timestamp: NonNull<AudioTimeStamp>,
    _: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> i32 {
    abort_on_panic(|| {
        // SAFETY: the refcon is the unit's speaker, which outlives it, and only this callback
        // uses it while the unit runs. The unit hands one buffer in the mono float format set,
        // of `mDataByteSize` bytes.
        let (speaker, buffer) = unsafe { (refcon.cast::<Speaker>().as_mut(), &mut (*data).mBuffers[0]) };
        let Some(start) = NonNull::new(buffer.mData.cast::<f32>()) else {
            return kAudio_ParamError;
        };
        let len = frames.min(buffer.mDataByteSize / SAMPLE_BYTES) as usize;
        // SAFETY: the buffer holds at least `len` samples, and the time stamp is valid during
        // the callback.
        let (samples, timestamp) = unsafe { (slice::from_raw_parts_mut(start.as_ptr(), len), timestamp.as_ref()) };
        speaker.pull(samples, instant(timestamp));
        0
    })
}

/// When a callback's first sample was captured or plays, by its time stamp's host time.
fn instant(timestamp: &AudioTimeStamp) -> Instant {
    let now = Instant::now();
    // The unit tells host times; were one missing, the callback runs close to its samples.
    if !timestamp.mFlags.contains(AudioTimeStampFlags::HostTimeValid) {
        return now;
    }
    let mut timebase = Timebase { numer: 1, denom: 1 };
    // SAFETY: it writes the timebase it is handed, which lives through the call. It fails only
    // for a null pointer, so its status says nothing.
    unsafe { mach_timebase_info(&mut timebase) };
    let duration = |ticks: u64| Duration::from_nanos((u128::from(ticks) * u128::from(timebase.numer) / u128::from(timebase.denom)) as u64);
    let host_now = mach_absolute_time();
    match timestamp.mHostTime.checked_sub(host_now) {
        Some(ahead) => now + duration(ahead),
        None => now - duration(host_now - timestamp.mHostTime),
    }
}

/// Runs a callback's body; a panic must not unwind into AudioToolbox, so it aborts.
fn abort_on_panic(body: impl FnOnce() -> i32) -> i32 {
    panic::catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| process::abort())
}

fn check(step: &'static str, status: i32) -> Result<(), Error> {
    if status == 0 { Ok(()) } else { Err(Error::Failed { step, status }) }
}
