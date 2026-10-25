//! The call's audio on iOS: its halves, idle while the audio session is inactive, playing on
//! the voice processing unit while it is active.

use std::mem;

use bukospeak_audio_ios::{Error, Failure, VoiceUnit};
use bukospeak_core::{Microphone, Speaker};

pub enum Audio {
    /// No call.
    None,
    Idle(Microphone, Speaker),
    Playing(VoiceUnit),
}

impl Audio {
    /// Plays the idle halves; playing already, or without a call, there is nothing to do.
    pub fn start(&mut self) -> Result<(), Error> {
        match mem::replace(self, Self::None) {
            Self::Idle(microphone, speaker) => match VoiceUnit::start(microphone, speaker) {
                Ok(unit) => {
                    *self = Self::Playing(unit);
                    Ok(())
                }
                Err(Failure { error, microphone, speaker }) => {
                    *self = Self::Idle(microphone, speaker);
                    Err(error)
                }
            },
            other => {
                *self = other;
                Ok(())
            }
        }
    }

    pub fn stop(&mut self) {
        *self = match mem::replace(self, Self::None) {
            Self::Playing(unit) => {
                let (microphone, speaker) = unit.stop();
                Self::Idle(microphone, speaker)
            }
            other => other,
        };
    }
}
