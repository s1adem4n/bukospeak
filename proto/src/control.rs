//! Control messages on the connection's first bidirectional stream, which the client opens.
//!
//! Each message is JSON behind its length as a big-endian u16. The client starts with a
//! [`ClientMessage::Hello`]; the server answers with a [`ServerMessage::Welcome`] or a
//! [`ServerMessage::Rejected`] and closes the connection after the latter.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::MemberId;

/// The longest name the server accepts, in characters.
pub const MAX_NAME_CHARS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: MemberId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello { version: u32, name: String, room_key: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The client joined as `you`; `members` are everyone in the channel, the client included.
    Welcome {
        you: MemberId,
        members: Vec<Member>,
    },
    Rejected {
        reason: Rejection,
    },
    Joined {
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
    #[error("a name has 1 to {MAX_NAME_CHARS} characters")]
    Name,
    #[error("the channel is full")]
    Full,
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

    #[tokio::test]
    async fn reads_what_it_writes() {
        let messages = [
            ServerMessage::Welcome { you: MemberId(1), members: vec![Member { id: MemberId(1), name: "member".into() }] },
            ServerMessage::Rejected { reason: Rejection::RoomKey },
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
