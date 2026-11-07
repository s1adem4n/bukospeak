// The tailnet tells the server who connects; TLS proves nothing here, so the server's
// certificate is made anew on each start and clients take any.

pub mod audio;
pub mod control;

pub const PROTOCOL_VERSION: u32 = 3;

pub const ALPN: &[u8] = b"bukospeak/1";

pub const SERVER_NAME: &str = "bukospeak";

pub const CLOSE_REPLACED: u32 = 1;

pub const KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(1);

pub const SILENT_AFTER: std::time::Duration = KEEP_ALIVE.saturating_mul(3);

pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

// TeamSpeak's 9987 is taken next to it.
pub const DEFAULT_PORT: u16 = 9988;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct MemberId(pub u16);
