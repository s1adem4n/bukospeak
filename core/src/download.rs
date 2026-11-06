//! A download the server offers, as NVIDIA's effects for a card: its files, fetched each on a
//! stream of its own, as the server keeps them, compressed with zstd.

use std::io::SeekFrom;
use std::path::Path;

use bukospeak_proto::control::{self, ClientMessage, File, FileRequest, ServerMessage};
use bukospeak_proto::{Fingerprint, PROTOCOL_VERSION};
use quinn::{Connection, Endpoint, SendStream};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::session::{Error, Opened};

/// A download, from the server's list of its files until it is closed or dropped.
pub struct Download {
    endpoint: Endpoint,
    connection: Connection,
    // Kept open: the server serves the download while the connection is.
    _send: SendStream,
    files: Vec<File>,
    fingerprint: Fingerprint,
}

impl Download {
    /// Asks the server, trusted by `pin`, or on first use if it is None, for the download `name`, and waits until it lists its files;
    /// `preparing` hears meanwhile how many of how many bytes the server has, if it prepares
    /// the download first. Fails if the server doesn't answer within
    /// [`JOIN_TIMEOUT`](crate::session::JOIN_TIMEOUT), or turns the client away, as when it
    /// doesn't offer the download or preparing it failed.
    pub async fn start(
        server: &str,
        room_key: String,
        pin: Option<Fingerprint>,
        name: String,
        mut preparing: impl FnMut(u64, u64),
    ) -> Result<Self, Error> {
        let download = ClientMessage::Download { version: PROTOCOL_VERSION, room_key, name };
        let (opened, mut answer) = Opened::with(server, pin, &download).await?;
        let Opened { endpoint, connection, send, mut control, fingerprint } = opened;
        let files = loop {
            match answer {
                ServerMessage::Preparing { done, total } => preparing(done, total),
                ServerMessage::Files { files } => break files,
                ServerMessage::Rejected { reason } => return Err(reason.into()),
                _ => return Err(Error::Protocol("a message that doesn't answer a Download")),
            }
            answer = match control.recv().await {
                Some(answer) => answer?,
                None => return Err(Error::Protocol("the control stream ended before the files")),
            };
        };
        Ok(Self { endpoint, connection, _send: send, files, fingerprint })
    }

    /// The download's files, as the server keeps them.
    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// The server's fingerprint, to pin for the next download or join.
    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Fetches `file`, which is one of [`files`](Self::files), to `to`, going on from where a
    /// fetch before stopped that left it there in part; `progress` hears how many of its bytes
    /// `to` holds. A file that came damaged is removed, for the next fetch to start over.
    pub async fn fetch(&self, file: &File, to: &Path, mut progress: impl FnMut(u64)) -> Result<(), Error> {
        let mut written = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(to).await?;
        let mut from = written.metadata().await?.len();
        if from > file.size {
            // Not the beginning of this file.
            written.set_len(0).await?;
            from = 0;
        }
        written.seek(SeekFrom::Start(from)).await?;
        progress(from);
        if from < file.size {
            let (mut send, mut recv) = self.connection.open_bi().await?;
            control::write(&mut send, &FileRequest { path: file.path.clone(), from }).await?;
            // The server ends the stream once it has sent the rest of the file.
            while let Some(chunk) = recv.read_chunk(usize::MAX, true).await? {
                written.write_all(&chunk.bytes).await?;
                from += chunk.bytes.len() as u64;
                progress(from);
            }
            written.flush().await?;
        }
        drop(written);
        let path = to.to_owned();
        let digest = tokio::task::spawn_blocking(move || control::digest(&mut std::fs::File::open(path)?))
            .await
            .expect("hashing a file doesn't panic")?;
        if digest != (file.size, file.sha256.clone()) {
            tokio::fs::remove_file(to).await?;
            return Err(Error::Damaged(file.path.clone()));
        }
        Ok(())
    }

    /// Tells the server the download is over.
    pub async fn close(self) {
        self.connection.close(0u32.into(), b"downloaded");
        self.endpoint.wait_idle().await;
    }
}
