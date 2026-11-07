use std::mem;

use bukospeak_core::Microphone;

use crate::output::Output;
use crate::voice_unit::{Error, Failure, VoiceUnit};

#[expect(clippy::large_enum_variant, reason = "an engine holds one, so its size doesn't matter")]
pub enum Audio {
    None,
    Idle(Microphone, Output),
    Playing(VoiceUnit),
}

impl Audio {
    pub fn start(&mut self) -> Result<(), Error> {
        match mem::replace(self, Self::None) {
            Self::Idle(microphone, output) => match VoiceUnit::start(microphone, output) {
                Ok(unit) => {
                    *self = Self::Playing(unit);
                    Ok(())
                }
                Err(failure) => {
                    let Failure { error, microphone, output } = *failure;
                    *self = Self::Idle(microphone, output);
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
                let (microphone, output) = unit.stop();
                Self::Idle(microphone, output)
            }
            other => other,
        };
    }
}
