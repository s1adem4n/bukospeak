//! The client side shared by the apps: joining the channel and exchanging its audio.

#[cfg_attr(not(test), expect(dead_code, reason = "the playout uses it next"))]
mod jitter;
mod pin;
mod session;
#[cfg(test)]
mod simulator;

pub use bukospeak_proto as proto;
pub use session::{AudioSender, Error, Event, JoinConfig, Session};
