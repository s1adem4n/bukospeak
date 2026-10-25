//! The client side shared by the apps: joining the channel and exchanging its audio.

mod pin;
mod session;

pub use bukospeak_proto as proto;
pub use session::{AudioSender, Error, Event, JoinConfig, Session};
