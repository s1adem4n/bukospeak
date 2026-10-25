//! Opus frames as QUIC datagrams.
//!
//! A frame is a 7-byte header and the Opus packet: the sender's id (u16) and the frame's index
//! (u32), both big-endian, then a byte with the last-frame flag in bit 7 and the level in bits
//! 0–6. A client sends its frames with id 0; the server stamps the sender's id over it before
//! forwarding, so it never parses the rest.

use std::time::Duration;

use crate::MemberId;

pub const SAMPLE_RATE: u32 = 48_000;

/// Every frame is this long; the frame index counts in these steps.
pub const FRAME_DURATION: Duration = Duration::from_millis(10);

/// Mono samples in a frame.
pub const FRAME_SAMPLES: usize = 480;

/// The longest Opus packet a receiver accepts: a 10 ms frame at 256 kbit/s, more than mono voice
/// ever needs. Receivers drop longer frames.
pub const MAX_OPUS_LEN: usize = 320;

pub const HEADER_LEN: usize = 7;

const LAST: u8 = 0x80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Who spoke, as the server stamped it; 0 on the way to the server.
    pub from: MemberId,
    /// The frame's place on the sender's timeline: it counts every frame the sender encoded,
    /// sent or not, so it is a timestamp in [`FRAME_DURATION`]s.
    pub index: u32,
    /// The last frame of a talkspurt: voice activation closed after it, and no frames follow
    /// until the member speaks again.
    pub last: bool,
    pub level: Level,
    pub opus: Vec<u8>,
}

/// A frame's loudness before encoding, in −dBov as RFC 6464 defines it: 0 is the loudest a
/// signal can be, 127 silence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(u8);

impl Level {
    pub const LOUDEST: Self = Self(0);
    pub const SILENT: Self = Self(127);

    /// A level of `minus_dbov` below full scale, quieter ones counting as silence.
    pub const fn new(minus_dbov: u8) -> Self {
        Self(if minus_dbov > Self::SILENT.0 { Self::SILENT.0 } else { minus_dbov })
    }

    /// The level of samples in −1.0..=1.0, by their RMS against a full-scale square wave.
    pub fn of(samples: &[f32]) -> Self {
        let power = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
        let minus_dbov = -10.0 * power.log10();
        // No samples and NaN samples give NaN, silence gives infinity; `as` saturates the latter.
        if minus_dbov.is_nan() { Self::SILENT } else { Self::new(minus_dbov.round() as u8) }
    }

    pub fn minus_dbov(self) -> u8 {
        self.0
    }
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(HEADER_LEN + self.opus.len());
        datagram.extend_from_slice(&self.from.0.to_be_bytes());
        datagram.extend_from_slice(&self.index.to_be_bytes());
        datagram.push(if self.last { LAST } else { 0 } | self.level.0);
        datagram.extend_from_slice(&self.opus);
        datagram
    }

    /// The frame in a datagram, or None if it is shorter than a header or its Opus packet is
    /// longer than [`MAX_OPUS_LEN`].
    pub fn decode(datagram: &[u8]) -> Option<Self> {
        let (header, opus) = datagram.split_at_checked(HEADER_LEN)?;
        if opus.len() > MAX_OPUS_LEN {
            return None;
        }
        Some(Self {
            from: MemberId(u16::from_be_bytes([header[0], header[1]])),
            index: u32::from_be_bytes([header[2], header[3], header[4], header[5]]),
            last: header[6] & LAST != 0,
            level: Level(header[6] & !LAST),
            opus: opus.to_vec(),
        })
    }
}

/// Writes the sender's id into a datagram's header in place; false if it has no header.
pub fn stamp(datagram: &mut [u8], from: MemberId) -> bool {
    if datagram.len() < HEADER_LEN {
        return false;
    }
    datagram[..2].copy_from_slice(&from.0.to_be_bytes());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes() {
        let frame = Frame { from: MemberId(3), index: 70_000, last: true, level: Level::new(42), opus: vec![1, 2, 3] };
        assert_eq!(Frame::decode(&frame.encode()), Some(frame));
    }

    #[test]
    fn stamps_the_sender() {
        let mut datagram = Frame { from: MemberId(0), index: 1, last: false, level: Level::SILENT, opus: vec![9] }.encode();
        assert!(stamp(&mut datagram, MemberId(42)));
        assert_eq!(Frame::decode(&datagram).unwrap().from, MemberId(42));
    }

    #[test]
    fn turns_away_a_short_or_oversized_datagram() {
        assert_eq!(Frame::decode(&[0; HEADER_LEN - 1]), None);
        assert!(!stamp(&mut [0; HEADER_LEN - 1], MemberId(1)));
        assert_eq!(Frame::decode(&[0; HEADER_LEN + MAX_OPUS_LEN + 1]), None);
    }

    #[test]
    fn measures_levels_in_dbov() {
        assert_eq!(Level::of(&[1.0, -1.0]), Level::LOUDEST);
        assert_eq!(Level::of(&[0.1, -0.1]), Level::new(20));
        assert_eq!(Level::of(&[0.0; 480]), Level::SILENT);
        assert_eq!(Level::of(&[f32::NAN]), Level::SILENT);
        assert_eq!(Level::of(&[]), Level::SILENT);
    }
}
