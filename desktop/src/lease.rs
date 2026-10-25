//! A value lent to an audio callback: it comes back when the stream drops the callback, which
//! every cpal backend does before dropping the stream returns.

use std::ops::{Deref, DerefMut};
use std::sync::mpsc;

pub struct Lease<T> {
    value: Option<T>,
    back: mpsc::Sender<T>,
}

impl<T> Lease<T> {
    pub fn new(value: T, back: mpsc::Sender<T>) -> Self {
        Self { value: Some(value), back }
    }
}

impl<T> Deref for Lease<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value.as_ref().expect("a lease holds its value until it is dropped")
    }
}

impl<T> DerefMut for Lease<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("a lease holds its value until it is dropped")
    }
}

impl<T> Drop for Lease<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            // Nobody waits for it once the audio itself is gone.
            let _ = self.back.send(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_comes_back_when_the_lease_is_dropped() {
        let (back, returned) = mpsc::channel();
        let mut lease = Lease::new(vec![1], back);
        lease.push(2);
        assert!(returned.try_recv().is_err());
        drop(lease);
        assert_eq!(returned.try_recv().unwrap(), [1, 2]);
    }
}
