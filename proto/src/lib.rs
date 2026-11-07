//! What the server and its clients say to each other.
//!
//! A client opens one QUIC connection over the tailnet: its first bidirectional stream carries
//! the control messages of [`control`], and its datagrams carry the Opus frames of [`audio`].
//!
//! The tailnet tells the server who connects, and that the server is who it says: TLS, which
//! QUIC can't do without, proves nothing here, so the server's certificate is made anew on each
//! start and clients take any.

pub mod audio;
pub mod control;

/// The version a client states in its Hello; the server turns away any other.
pub const PROTOCOL_VERSION: u32 = 3;

/// The ALPN protocol of the QUIC connection.
pub const ALPN: &[u8] = b"bukospeak/1";

/// The name the server's certificate is issued to and clients connect to; they take any
/// certificate, so it only has to agree.
pub const SERVER_NAME: &str = "bukospeak";

/// The code the server closes a member's connection with once another connection of the
/// member's device took its place: that one is the member now, and this one mustn't take it
/// back.
pub const REPLACED: u32 = 1;

/// How often a client sends at least a keep-alive, so the server notices soon when nothing
/// comes from it.
pub const KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(1);

/// Three keep-alives missed: a connection nothing came over this long shows reconnecting on
/// either side, long before it times out.
pub const SILENT_AFTER: std::time::Duration = KEEP_ALIVE.saturating_mul(3);

/// A connection that stays silent this long is gone, on either side; clients send keep-alives
/// well within it.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The UDP port the server listens on unless told otherwise. TeamSpeak's 9987 is taken next to it.
pub const DEFAULT_PORT: u16 = 9988;

/// A member's id, unique while the server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct MemberId(pub u16);
