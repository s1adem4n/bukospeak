//! What the server and its clients say to each other.
//!
//! A client opens one QUIC connection: its first bidirectional stream carries the control
//! messages of [`control`], and its datagrams carry the Opus frames of [`audio`].

pub mod audio;
pub mod control;
mod fingerprint;

pub use fingerprint::{Fingerprint, ParseFingerprintError};

/// The version a client states in its Hello; the server turns away any other.
pub const PROTOCOL_VERSION: u32 = 2;

/// The ALPN protocol of the QUIC connection.
pub const ALPN: &[u8] = b"bukospeak/1";

/// The name the server's self-signed certificate is issued to. Clients trust the certificate by
/// its pinned fingerprint, not by this name.
pub const SERVER_NAME: &str = "bukospeak";

/// A connection that stays silent this long is gone, on either side; clients send keep-alives
/// well within it.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The UDP port the server listens on unless told otherwise. TeamSpeak's 9987 is taken next to it.
pub const DEFAULT_PORT: u16 = 9988;

/// A member's id, unique while the server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct MemberId(pub u16);
