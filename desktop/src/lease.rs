//! A call's half lent to an audio callback: it comes back when the stream drops the callback,
//! which every cpal backend does before dropping the stream returns.

use std::ops::{Deref, DerefMut};
use std::sync::mpsc;

use bukospeak_core::{Microphone, Speaker};

/// A call's half, which a stream plays until it stops.
pub trait Half {
    /// Tells the half its stream stopped calling it.
    fn stopped(&mut self);
}

impl Half for Microphone {
    fn stopped(&mut self) {
        Microphone::stopped(self);
    }
}

impl Half for Speaker {
    fn stopped(&mut self) {
        Speaker::stopped(self);
    }
}

pub struct Lease<T: Half> {
    value: Option<T>,
    back: mpsc::Sender<T>,
}

impl<T: Half> Lease<T> {
    pub fn new(value: T, back: mpsc::Sender<T>) -> Self {
        Self { value: Some(value), back }
    }
}

impl<T: Half> Deref for Lease<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value.as_ref().expect("a lease holds its value until it is dropped")
    }
}

impl<T: Half> DerefMut for Lease<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("a lease holds its value until it is dropped")
    }
}

impl<T: Half> Drop for Lease<T> {
    fn drop(&mut self) {
        if let Some(mut value) = self.value.take() {
            value.stopped();
            // Nobody waits for it once the audio itself is gone.
            let _ = self.back.send(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Played {
        frames: u32,
        stopped: bool,
    }

    impl Half for Played {
        fn stopped(&mut self) {
            self.stopped = true;
        }
    }

    #[test]
    fn the_half_comes_back_stopped_when_the_lease_is_dropped() {
        let (back, returned) = mpsc::channel();
        let mut lease = Lease::new(Played { frames: 0, stopped: false }, back);
        lease.frames += 1;
        assert!(returned.try_recv().is_err());
        drop(lease);
        assert_eq!(returned.try_recv().unwrap(), Played { frames: 1, stopped: true });
    }
}
