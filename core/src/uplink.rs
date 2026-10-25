//! The thread that sends the microphone's frames: it sleeps until the capture callback wakes
//! it, since waking a thread is all a realtime callback may do.

use std::io;
use std::thread::{self, Thread};

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::Frame;
use quinn::SendDatagramError;
use rtrb::Consumer;

use crate::capture::Encoded;
use crate::session::AudioSender;

/// Starts the uplink; it ends when the microphone is gone or the connection is lost.
pub fn spawn(mut frames: Consumer<Encoded>, sender: AudioSender) -> io::Result<Thread> {
    let uplink = thread::Builder::new().name("bukospeak-uplink".into()).spawn(move || {
        loop {
            while let Ok(frame) = frames.pop() {
                let frame =
                    Frame { from: MemberId(0), index: frame.index, last: frame.last, level: frame.level, opus: frame.opus().to_vec() };
                // A frame the path can't take now is lost, which the receivers conceal.
                if let Err(SendDatagramError::ConnectionLost(_)) = sender.send(&frame) {
                    return;
                }
            }
            if frames.is_abandoned() {
                return;
            }
            thread::park();
        }
    })?;
    Ok(uplink.thread().clone())
}
