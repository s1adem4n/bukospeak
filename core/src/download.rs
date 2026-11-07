use std::io::SeekFrom;
use std::path::Path;

use bukospeak_proto::PROTOCOL_VERSION;
use bukospeak_proto::control::{self, ClientMessage, File, FileRequest, ServerMessage};
use quinn::{Connection, Endpoint, SendStream};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::session::{Error, Opened};

pub struct Download {
    endpoint: Endpoint,
    connection: Connection,
    // The server serves the download while the stream is open.
    _send: SendStream,
    files: Vec<File>,
}

impl Download {
    pub async fn start(server: &str, name: String, mut preparing: impl FnMut(u64, u64)) -> Result<Self, Error> {
        let (opened, mut answer) = Opened::with(server, &ClientMessage::Download { version: PROTOCOL_VERSION, name }).await?;
        let Opened { endpoint, connection, send, mut control } = opened;
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
        Ok(Self { endpoint, connection, _send: send, files })
    }

    pub fn files(&self) -> &[File] {
        &self.files
    }

    pub async fn fetch(&self, file: &File, to: &Path, mut progress: impl FnMut(u64)) -> Result<(), Error> {
        let mut written = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(to).await?;
        let mut from = written.metadata().await?.len();
        if from > file.size {
            written.set_len(0).await?;
            from = 0;
        }
        written.seek(SeekFrom::Start(from)).await?;
        progress(from);
        if from < file.size {
            let (mut send, mut recv) = self.connection.open_bi().await?;
            control::write(&mut send, &FileRequest { path: file.path.clone(), from }).await?;
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

    pub async fn close(self) {
        self.connection.close(0u32.into(), b"downloaded");
        self.endpoint.wait_idle().await;
    }
}
