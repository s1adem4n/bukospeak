use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::MemberId;
pub const SOUNDS: &str = "sounds";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: MemberId,
    pub name: String,
    pub muted: bool,
    pub reconnecting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello { version: u32, muted: bool },
    Watch { version: u32 },
    Download { version: u32, name: String },
    Mute { muted: bool },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest {
    pub path: String,
    pub from: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
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
    Welcome { you: MemberId, members: Vec<Member> },
    Present { members: Vec<Member> },
    Rejected { reason: Rejection },
    Preparing { done: u64, total: u64 },
    Files { files: Vec<File> },
    Joined { member: Member },
    Changed { member: Member },
    Left { id: MemberId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    #[error("the server speaks another protocol version")]
    Version,
    #[error("the server's tailnet doesn't know this device")]
    Stranger,
    #[error("the server couldn't ask its tailnet who this device is")]
    Unidentified,
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
            },
            ServerMessage::Present { members: Vec::new() },
            ServerMessage::Rejected { reason: Rejection::Stranger },
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
