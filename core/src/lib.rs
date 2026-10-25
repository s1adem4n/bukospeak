//! The client side shared by the apps: joining the channel and exchanging its audio.

#[cfg_attr(not(test), expect(dead_code, reason = "the engine uses it next, through the playout"))]
mod jitter;
mod pin;
#[cfg_attr(not(test), expect(dead_code, reason = "the engine uses it next"))]
mod playout;
mod session;
#[cfg(test)]
mod simulator;

pub use bukospeak_proto as proto;
pub use session::{AudioSender, Error, Event, JoinConfig, Session};
