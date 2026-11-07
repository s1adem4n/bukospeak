use std::time::Duration;

use crate::MemberId;

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_DURATION: Duration = Duration::from_millis(10);
pub const FRAME_SAMPLES: usize = 480;
// A 10 ms frame at 256 kbit/s, more than mono voice ever needs.
pub const MAX_OPUS_LEN: usize = 320;

pub const HEADER_LEN: usize = 7;

const LAST: u8 = 0x80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub from: MemberId,
    pub index: u32,
    pub last: bool,
    pub level: Level,
    pub opus: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(u8);

impl Level {
    pub const LOUDEST: Self = Self(0);
    pub const SILENT: Self = Self(127);
    pub const fn new(minus_dbov: u8) -> Self {
        Self(if minus_dbov > Self::SILENT.0 { Self::SILENT.0 } else { minus_dbov })
    }
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
