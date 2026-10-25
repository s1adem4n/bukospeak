//! Opus frames as QUIC datagrams.
//!
//! A frame is a 7-byte header and the Opus packet: the sender's id (u16), the frame's sequence
//! number (u32), both big-endian, and a flags byte. A client sends its frames with id 0; the
//! server stamps the sender's id over it before forwarding, so it never parses the rest.

use crate::MemberId;

pub const HEADER_LEN: usize = 7;

/// Set on the last frame of a talkspurt: voice activation closed after it, and no frames follow
/// until the member speaks again.
pub const FLAG_LAST: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Who spoke, as the server stamped it; 0 on the way to the server.
    pub from: MemberId,
    /// Counts up by one per frame, so a receiver sees losses and reorders.
    pub seq: u32,
    pub flags: u8,
    pub opus: Vec<u8>,
}

impl Frame {
    pub fn is_last(&self) -> bool {
        self.flags & FLAG_LAST != 0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(HEADER_LEN + self.opus.len());
        datagram.extend_from_slice(&self.from.0.to_be_bytes());
        datagram.extend_from_slice(&self.seq.to_be_bytes());
        datagram.push(self.flags);
        datagram.extend_from_slice(&self.opus);
        datagram
    }

    /// The frame in a datagram, or None if it is shorter than a header.
    pub fn decode(datagram: &[u8]) -> Option<Self> {
        let (header, opus) = datagram.split_at_checked(HEADER_LEN)?;
        Some(Self {
            from: MemberId(u16::from_be_bytes([header[0], header[1]])),
            seq: u32::from_be_bytes([header[2], header[3], header[4], header[5]]),
            flags: header[6],
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
        let frame = Frame { from: MemberId(3), seq: 70_000, flags: FLAG_LAST, opus: vec![1, 2, 3] };
        assert_eq!(Frame::decode(&frame.encode()), Some(frame));
    }

    #[test]
    fn stamps_the_sender() {
        let mut datagram = Frame { from: MemberId(0), seq: 1, flags: 0, opus: vec![9] }.encode();
        assert!(stamp(&mut datagram, MemberId(42)));
        assert_eq!(Frame::decode(&datagram).unwrap().from, MemberId(42));
    }

    #[test]
    fn turns_away_a_short_datagram() {
        assert_eq!(Frame::decode(&[0; 6]), None);
        assert!(!stamp(&mut [0; 6], MemberId(1)));
    }
}
