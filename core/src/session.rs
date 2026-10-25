use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_proto::control::{self, ClientMessage, Member, Rejection, ServerMessage};
use bukospeak_proto::{ALPN, Fingerprint, MemberId, PROTOCOL_VERSION, SERVER_NAME, audio};
use bytes::Bytes;
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use tokio::sync::mpsc;

use crate::pin::PinVerifier;

/// Often enough to keep NAT and Tailscale paths open, and well within the server's idle timeout.
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

pub struct JoinConfig {
    /// The server as `host:port`.
    pub server: String,
    pub name: String,
    pub room_key: String,
    /// The fingerprint pinned on an earlier join, or None to trust the server on first use.
    pub pin: Option<Fingerprint>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} doesn't resolve to an address")]
    Resolve(String),
    #[error("the server's certificate changed: pinned {pinned}, presented {presented}")]
    FingerprintChanged { pinned: Fingerprint, presented: Fingerprint },
    #[error("the server turned the join away")]
    Rejected(#[from] Rejection),
    #[error(transparent)]
    Connect(#[from] quinn::ConnectError),
    #[error(transparent)]
    Connection(#[from] quinn::ConnectionError),
    #[error(transparent)]
    Control(#[from] control::Error),
    #[error("the server broke the protocol: {0}")]
    Protocol(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What happens in the channel after the join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Joined(Member),
    Left(MemberId),
    Audio(audio::Frame),
}

/// A member's place in the channel, from the join until [`Session::leave`] or the connection is lost.
pub struct Session {
    endpoint: Endpoint,
    connection: Connection,
    // Kept open: finishing it tells the server the member left.
    _send: SendStream,
    control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
    you: MemberId,
    members: Vec<Member>,
    fingerprint: Fingerprint,
}

impl Session {
    pub async fn join(config: JoinConfig) -> Result<Self, Error> {
        let server = tokio::net::lookup_host(&config.server).await?.next().ok_or_else(|| Error::Resolve(config.server.clone()))?;
        let endpoint = Endpoint::client(unspecified_like(server))?;

        let (client, presented) = client_config(config.pin);

        let connection = endpoint.connect_with(client, server, SERVER_NAME)?.await.map_err(|error| {
            let presented = *presented.lock().expect("the fingerprint lock is never poisoned");
            match (config.pin, presented) {
                (Some(pinned), Some(presented)) if pinned != presented => Error::FingerprintChanged { pinned, presented },
                _ => error.into(),
            }
        })?;
        let fingerprint =
            presented.lock().expect("the fingerprint lock is never poisoned").expect("a connected server presented a certificate");

        let (mut send, mut recv) = connection.open_bi().await?;
        let hello = ClientMessage::Hello { version: PROTOCOL_VERSION, name: config.name, room_key: config.room_key };
        control::write(&mut send, &hello).await?;
        let (you, members) = match control::read(&mut recv).await? {
            Some(ServerMessage::Welcome { you, members }) => (you, members),
            Some(ServerMessage::Rejected { reason }) => {
                connection.close(0u32.into(), b"rejected");
                return Err(reason.into());
            }
            Some(_) => return Err(Error::Protocol("the first message isn't a Welcome")),
            None => return Err(Error::Protocol("the control stream ended before a Welcome")),
        };

        let (forward, control) = mpsc::unbounded_channel();
        tokio::spawn(read_control(recv, forward));
        Ok(Self { endpoint, connection, _send: send, control, you, members, fingerprint })
    }

    pub fn you(&self) -> MemberId {
        self.you
    }

    /// Everyone in the channel at the join, this member included; [`Event`]s tell what changes.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The server's fingerprint, to pin for the next join.
    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// A handle that sends this member's audio, for the thread that encodes it.
    pub fn audio_sender(&self) -> AudioSender {
        AudioSender { connection: self.connection.clone() }
    }

    /// The next event; an error means the session is over.
    pub async fn next_event(&mut self) -> Result<Event, Error> {
        loop {
            tokio::select! {
                message = self.control.recv() => match message {
                    Some(Ok(ServerMessage::Joined { member })) => return Ok(Event::Joined(member)),
                    Some(Ok(ServerMessage::Left { id })) => return Ok(Event::Left(id)),
                    Some(Ok(ServerMessage::Welcome { .. } | ServerMessage::Rejected { .. })) => {
                        return Err(Error::Protocol("a second Welcome or a Rejected after the join"));
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => return Err(self.connection.closed().await.into()),
                },
                datagram = self.connection.read_datagram() => {
                    // A datagram without a header is noise; the server only relays frames.
                    if let Some(frame) = audio::Frame::decode(&datagram?) {
                        return Ok(Event::Audio(frame));
                    }
                }
            }
        }
    }

    pub async fn leave(self) {
        self.connection.close(0u32.into(), b"left");
        self.endpoint.wait_idle().await;
    }
}

#[derive(Clone)]
pub struct AudioSender {
    connection: Connection,
}

impl AudioSender {
    /// Sends a frame, its `from` left for the server to stamp. A frame the path can't take now is
    /// dropped, as a late one would be.
    pub fn send(&self, frame: &audio::Frame) -> Result<(), quinn::SendDatagramError> {
        self.connection.send_datagram(Bytes::from(frame.encode()))
    }
}

/// TLS 1.3 that trusts the server by the `pin`, and keep-alives that hold the path open; the slot
/// receives the fingerprint the server presents.
fn client_config(pin: Option<Fingerprint>) -> (quinn::ClientConfig, Arc<Mutex<Option<Fingerprint>>>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (verifier, presented) = PinVerifier::new(pin, &provider);
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("15 s is a valid idle timeout")));
    let tls = QuicClientConfig::try_from(tls).expect("the config has TLS 1.3 and no early data");
    let mut client = quinn::ClientConfig::new(Arc::new(tls));
    client.transport_config(Arc::new(transport));
    (client, presented)
}

/// Moves the server's control messages into a channel, which `next_event` can wait on beside
/// datagrams: a read cut off mid-message would lose its part.
async fn read_control(mut recv: RecvStream, forward: mpsc::UnboundedSender<Result<ServerMessage, control::Error>>) {
    loop {
        match control::read(&mut recv).await {
            Ok(Some(message)) => {
                if forward.send(Ok(message)).is_err() {
                    return;
                }
            }
            Ok(None) => return,
            Err(error) => {
                let _ = forward.send(Err(error));
                return;
            }
        }
    }
}

/// The any-address of the server's family, so the endpoint can reach it.
fn unspecified_like(server: SocketAddr) -> SocketAddr {
    match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    }
}
