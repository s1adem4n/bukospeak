//! The client side shared by the apps: the engine, and the session and audio processing it runs.

mod capture;
mod conditioner;
mod engine;
mod gate;
mod jitter;
mod limiter;
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
pub use conditioner::Conditioning;
pub use engine::{Engine, Event, Joined, Listener};
pub use microphone::Microphone;
pub use session::{Error, JoinConfig};
pub use speaker::Speaker;
