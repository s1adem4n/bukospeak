//! The client side shared by the apps: the engine, and the session and audio processing it runs.

mod capture;
mod engine;
mod gate;
mod jitter;
mod microphone;
mod pin;
mod playout;
pub mod session;
mod speaker;
mod speaking;
mod uplink;

#[cfg(test)]
mod signal;
#[cfg(test)]
mod simulator;

pub use bukospeak_proto as proto;
pub use engine::{Engine, Event, Joined, Listener};
pub use microphone::Microphone;
pub use session::{Error, JoinConfig};
pub use speaker::Speaker;
