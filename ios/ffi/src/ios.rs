use std::mem;

use crate::voice_unit::{Error, Failure, VoiceUnit};
use bukospeak_core::{Microphone, Speaker};

#[expect(clippy::large_enum_variant, reason = "an engine holds one, so its size doesn't matter")]
pub enum Audio {
    None,
    Idle(Microphone, Speaker),
    Playing(VoiceUnit),
}

impl Audio {
    pub fn start(&mut self) -> Result<(), Error> {
        match mem::replace(self, Self::None) {
            Self::Idle(microphone, speaker) => match VoiceUnit::start(microphone, speaker) {
                Ok(unit) => {
                    *self = Self::Playing(unit);
                    Ok(())
                }
                Err(failure) => {
                    let Failure { error, microphone, speaker } = *failure;
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
