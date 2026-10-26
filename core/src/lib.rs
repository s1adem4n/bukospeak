//! The client side shared by the apps: the engine, and the session and audio processing it runs.

mod capture;
mod channel;
mod conditioner;
#[cfg(feature = "deepfilter")]
mod deep_filter;
mod echo;
mod engine;
mod gate;
#[cfg(any(test, feature = "harness"))]
pub mod harness;
mod jitter;
mod limiter;
mod microphone;
mod pin;
mod playout;
pub mod session;
mod speaker;
mod speaking;
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
pub use engine::{Engine, Joined, Listener, Update};
pub use microphone::Microphone;
pub use session::{Error, JoinConfig};
pub use speaker::Speaker;
