//! Control messages on the connection's first bidirectional stream, which the client opens.
//!
//! Each message is JSON behind its length as a big-endian u16. The client starts with a
//! [`ClientMessage::Hello`], which the server answers with a [`ServerMessage::Welcome`]; with a
//! [`ClientMessage::Watch`], which it answers with a [`ServerMessage::Present`]; or with a
//! [`ClientMessage::Download`], which it answers with [`ServerMessage::Preparing`] while it
//! prepares the download and then with its [`ServerMessage::Files`], each of which the client
//! asks for by a [`FileRequest`] on a stream of its own. Or the server answers with a
//! [`ServerMessage::Rejected`], after which it closes the connection.
//!
//! A member whose connection was lost joins again with the [`ResumeToken`] of its Welcome, and
//! takes its own place: the others see it [`reconnecting`](Member::reconnecting) meanwhile, not
//! leave and join.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::MemberId;

/// The longest name the server accepts, in characters.
pub const MAX_NAME_CHARS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: MemberId,
    pub name: String,
    pub muted: bool,
    /// The member's connection was lost, and the server holds its place for it to come back.
    pub reconnecting: bool,
}

/// The secret that lets a member whose connection was lost take its place again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeToken(pub [u8; 16]);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Joins the channel, or takes the place of the session `resume` names if the server still
    /// holds it.
    Hello { version: u32, name: String, room_key: String, muted: bool, resume: Option<ResumeToken> },
    /// Watches who is in the channel without joining it: after the Present, the server tells
    /// who joins, changes and leaves, until the client closes the connection.
    Watch { version: u32, room_key: String },
    /// Asks for the files of a download the server offers, by its `name`.
    Download { version: u32, room_key: String, name: String },
    /// Mutes the member, or unmutes it, after the Hello.
    Mute { muted: bool },
}

/// Asks for a file of a download on a bidirectional stream of its own, after the download's
/// Files: the server sends the file from its byte `from` on, and finishes the stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest {
    pub path: String,
    pub from: u64,
}

/// A file of a download, as the server sends it: compressed with zstd.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    /// Where the file goes, relative to the download's folder, its folders parted by `/`.
    pub path: String,
    pub size: u64,
    /// Its SHA-256, in lowercase hex.
    pub sha256: String,
}

/// How many bytes `read` gives, and their SHA-256 as a [`File`] tells it.
pub fn digest(read: &mut (impl std::io::Read + ?Sized)) -> std::io::Result<(u64, String)> {
    use sha2::{Digest, Sha256};

    let (mut hasher, mut size, mut buffer) = (Sha256::new(), 0, vec![0; 1 << 16]);
    loop {
        match read.read(&mut buffer)? {
            0 => return Ok((size, hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())),
            read => {
                hasher.update(&buffer[..read]);
                size += read as u64;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The client joined as `you`; `members` are everyone in the channel, the client included.
    Welcome {
        you: MemberId,
        members: Vec<Member>,
        resume: ResumeToken,
    },
    /// The client watches the channel; `members` are everyone in it.
    Present {
        members: Vec<Member>,
    },
    Rejected {
        reason: Rejection,
    },
    /// The server prepares the download, and has `done` of its `total` bytes; Files follow once
    /// it is ready, or a Rejected if preparing it failed.
    Preparing {
        done: u64,
        total: u64,
    },
    /// The download's files.
    Files {
        files: Vec<File>,
    },
    Joined {
        member: Member,
    },
    /// A member muted or unmuted, or is reconnecting or back.
    Changed {
        member: Member,
    },
    Left {
        id: MemberId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    #[error("the server speaks another protocol version")]
    Version,
    #[error("the room key is wrong")]
    RoomKey,
    #[error("a name has 1 to {MAX_NAME_CHARS} characters, none of them control characters")]
    Name,
    #[error("the channel is full")]
    Full,
    #[error("the server has no such download to offer")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("a message is at most 65535 bytes")]
    TooLong,
}

pub async fn write<W: AsyncWrite + Unpin>(stream: &mut W, message: &impl Serialize) -> Result<(), Error> {
    let json = serde_json::to_vec(message)?;
    let len = u16::try_from(json.len()).map_err(|_| Error::TooLong)?;
    let mut frame = Vec::with_capacity(2 + json.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&json);
    stream.write_all(&frame).await?;
    Ok(())
}

/// The next message, or None once the stream ended between messages.
pub async fn read<R: AsyncRead + Unpin, M: for<'de> Deserialize<'de>>(stream: &mut R) -> Result<Option<M>, Error> {
    let mut len = [0; 2];
    match stream.read_exact(&mut len).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut json = vec![0; u16::from_be_bytes(len).into()];
    stream.read_exact(&mut json).await?;
    Ok(Some(serde_json::from_slice(&json)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_files_digest_is_its_size_and_sha256_in_hex() {
        let (size, sha256) = digest(&mut &b"abc"[..]).unwrap();
        assert_eq!((size, sha256.as_str()), (3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    }

    #[tokio::test]
    async fn reads_what_it_writes() {
        let messages = [
            ServerMessage::Welcome {
                you: MemberId(1),
                members: vec![Member { id: MemberId(1), name: "member".into(), muted: false, reconnecting: false }],
                resume: ResumeToken([7; 16]),
            },
            ServerMessage::Present { members: Vec::new() },
            ServerMessage::Rejected { reason: Rejection::RoomKey },
            ServerMessage::Preparing { done: 1, total: 2 },
            ServerMessage::Files { files: vec![File { path: "bin/first.dll".into(), size: 3, sha256: "00".repeat(32) }] },
        ];
        let mut stream = Vec::new();
        for message in &messages {
            write(&mut stream, message).await.unwrap();
        }
        let mut stream = stream.as_slice();
        for message in messages {
            assert_eq!(read::<_, ServerMessage>(&mut stream).await.unwrap(), Some(message));
        }
        assert_eq!(read::<_, ServerMessage>(&mut stream).await.unwrap(), None);
    }
}
