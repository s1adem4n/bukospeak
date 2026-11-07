//! The client side shared by the apps: the engine, and the session and audio processing it runs.

mod capture;
mod channel;
mod conditioner;
#[cfg(feature = "deepfilter")]
mod deep_filter;
pub mod download;
mod echo;
mod engine;
mod gate;
#[cfg(any(test, feature = "harness"))]
pub mod harness;
mod jitter;
mod limiter;
mod meter;
mod microphone;
mod playout;
pub mod session;
mod speaker;
mod speaking;
mod suppression;
mod uplink;

/// Lets tests assert that the realtime paths don't allocate.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[cfg(test)]
mod signal;
#[cfg(test)]
mod simulator;

pub use bukospeak_proto as proto;
pub use channel::{Announcement, Channel, Lost, State};
pub use conditioner::Conditioning;
pub use engine::{Engine, Joined, Listener, MAX_VOLUME, Update};
pub use meter::Meter;
pub use microphone::Microphone;
pub use session::Error;
pub use speaker::Speaker;
pub use suppression::{MAX_DELAY, MakeSuppressor, Suppression, Suppressor};
