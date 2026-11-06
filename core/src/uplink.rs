//! The thread that processes the microphone's frames and sends them: it sleeps until the
//! capture callback wakes it, since waking a thread is all a realtime callback may do.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, Thread};
use std::time::Instant;

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::Frame;
use rtrb::Consumer;

use crate::capture::{Capture, Captured};
use crate::session::AudioSender;
use crate::speaking::Speaking;
use crate::suppression::Stage;

/// A stage made for the suppression chosen as the `choice`-th.
pub struct Made {
    pub choice: u64,
    pub stage: Stage,
}

/// Starts the uplink; it ends when the microphone is gone. It suppresses noise with each stage
/// `made` hands it that is for the latest suppression `chosen`, from the next frame on.
pub fn spawn(
    mut captured: Consumer<Captured>,
    mut capture: Capture,
    made: mpsc::Receiver<Made>,
    chosen: Arc<AtomicU64>,
    sender: AudioSender,
    speaking: Arc<Speaking>,
) -> io::Result<Thread> {
    let uplink = thread::Builder::new().name("bukospeak-uplink".into()).spawn(move || {
        loop {
            while let Ok(next) = captured.pop() {
                // Stages may be made out of order; one for an earlier choice is dropped.
                for Made { choice, stage } in made.try_iter() {
                    if choice == chosen.load(Ordering::Relaxed) {
                        capture.set_stage(stage);
                    }
                }
                let mut last_sent = None;
                // libopus fails only on arguments the fixed frame size rules out; such a frame
                // is lost.
                let _ = capture.take(&next, Instant::now(), |encoded| {
                    let frame = Frame {
                        from: MemberId(0),
                        index: encoded.index,
                        last: encoded.last,
                        level: encoded.level,
                        opus: encoded.opus().to_vec(),
                    };
                    // A frame the path can't take now is lost, which the receivers conceal; while
                    // the connection is lost they all are, until the engine joined again or, if it
                    // couldn't, the microphone of the call that ended is dropped.
                    let _ = sender.send(&frame);
                    last_sent = Some(encoded.last);
                });
                speaking.set_you(last_sent == Some(false));
            }
            if captured.is_abandoned() {
                speaking.set_you(false);
                return;
            }
            thread::park();
        }
    })?;
    Ok(uplink.thread().clone())
}
