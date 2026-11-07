use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_proto::control::{self, ClientMessage, Member, Rejection, ServerMessage};
use bukospeak_proto::{ALPN, CLOSE_REPLACED, IDLE_TIMEOUT, KEEP_ALIVE, MemberId, PROTOCOL_VERSION, SERVER_NAME, audio};
use bytes::Bytes;
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{Connection, ConnectionError, Endpoint, RecvStream, SendStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::sync::mpsc;

/// How long a join may take, from resolving the server to its Welcome.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} doesn't resolve to an address")]
    Resolve(String),
    #[error("the server turned the client away")]
    Rejected(#[from] Rejection),
    #[error("this device joined again elsewhere, which took its place")]
    Replaced,
    #[error(transparent)]
    Connect(#[from] quinn::ConnectError),
    #[error(transparent)]
    Connection(ConnectionError),
    #[error(transparent)]
    Control(#[from] control::Error),
    #[error(transparent)]
    Read(quinn::ReadError),
    #[error("the server broke the protocol: {0}")]
    Protocol(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the server didn't answer within {} s", JOIN_TIMEOUT.as_secs())]
    TimedOut,
    #[error("{0} came other than the server listed it")]
    Damaged(String),
}

impl From<ConnectionError> for Error {
    fn from(error: ConnectionError) -> Self {
        match error {
            ConnectionError::ApplicationClosed(close) if close.error_code.into_inner() == u64::from(CLOSE_REPLACED) => Self::Replaced,
            error => Self::Connection(error),
        }
    }
}

impl From<quinn::ReadError> for Error {
    fn from(error: quinn::ReadError) -> Self {
        match error {
            quinn::ReadError::ConnectionLost(error) => error.into(),
            error => Self::Read(error),
        }
    }
}

/// What happens in the channel after the join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Joined(Member),
    /// A member muted or unmuted, or is reconnecting or back; this member too.
    Changed(Member),
    Left(MemberId),
    Audio(audio::Frame),
}

/// A member's place in the channel, from the join until [`Session::leave`] or the connection is lost.
pub struct Session {
    endpoint: Endpoint,
    connection: Connection,
    // Kept open: finishing it tells the server the member left.
    send: SendStream,
    control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
    you: MemberId,
    members: Vec<Member>,
}

impl Session {
    /// Joins the channel of `server`, as `host:port`, muted or not, in the place it still holds
    /// for this device if it does; fails if the server doesn't welcome this member within
    /// [`JOIN_TIMEOUT`].
    pub async fn join(server: &str, muted: bool) -> Result<Self, Error> {
        let (opened, answer) = Opened::with(server, &ClientMessage::Hello { version: PROTOCOL_VERSION, muted }).await?;
        let ServerMessage::Welcome { you, members } = answer else {
            return Err(Error::Protocol("the first message isn't a Welcome"));
        };
        let Opened { endpoint, connection, send, control } = opened;
        Ok(Self { endpoint, connection, send, control, you, members })
    }

    pub fn you(&self) -> MemberId {
        self.you
    }

    /// Everyone in the channel at the join, this member included; [`Event`]s tell what changes.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// A handle that sends this member's audio, for the thread that encodes it.
    pub fn audio_sender(&self) -> AudioSender {
        AudioSender { connection: Arc::new(Mutex::new(self.connection.clone())) }
    }

    /// How many datagrams came over the connection so far, keep-alives' acknowledgements among
    /// them: while it stands still, nothing comes from the server.
    pub fn received(&self) -> u64 {
        self.connection.stats().udp_rx.datagrams
    }

    /// Mutes or unmutes this member for the others to see; the microphone's silence is its own.
    pub async fn set_muted(&mut self, muted: bool) -> Result<(), Error> {
        Ok(control::write(&mut self.send, &ClientMessage::Mute { muted }).await?)
    }

    /// The next event; an error means the session is over.
    pub async fn next_event(&mut self) -> Result<Event, Error> {
        loop {
            tokio::select! {
                message = self.control.recv() => match message {
                    Some(Ok(ServerMessage::Joined { member })) => return Ok(Event::Joined(member)),
                    Some(Ok(ServerMessage::Changed { member })) => return Ok(Event::Changed(member)),
                    Some(Ok(ServerMessage::Left { id })) => return Ok(Event::Left(id)),
                    Some(Ok(
                        ServerMessage::Welcome { .. }
                        | ServerMessage::Present { .. }
                        | ServerMessage::Rejected { .. }
                        | ServerMessage::Preparing { .. }
                        | ServerMessage::Files { .. },
                    )) => return Err(Error::Protocol("a message after the join that only answers a client's first")),
                    // A read that failed as the connection closed fails for the reason it closed.
                    Some(Err(error)) => return Err(self.connection.close_reason().map_or(error.into(), Error::from)),
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

/// Who is in the channel, watched without joining it, from the start until it is dropped or the
/// connection is lost.
pub struct Watch {
    // Kept for the connection: dropping both closes it, which ends the watch at the server.
    _endpoint: Endpoint,
    _connection: Connection,
    _send: SendStream,
    control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
    members: Vec<Member>,
}

impl Watch {
    /// Starts watching `server`, or fails if it doesn't answer within [`JOIN_TIMEOUT`].
    pub async fn start(server: &str) -> Result<Self, Error> {
        let (opened, answer) = Opened::with(server, &ClientMessage::Watch { version: PROTOCOL_VERSION }).await?;
        let ServerMessage::Present { members } = answer else {
            return Err(Error::Protocol("the first message isn't a Present"));
        };
        let Opened { endpoint, connection, send, control, .. } = opened;
        Ok(Self { _endpoint: endpoint, _connection: connection, _send: send, control, members })
    }

    /// Everyone in the channel, in the order they came.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Waits until someone joins, changes or leaves, for [`members`](Self::members) to tell who
    /// is there then; an error means the watch is over.
    pub async fn changed(&mut self) -> Result<(), Error> {
        match self.control.recv().await {
            Some(Ok(ServerMessage::Joined { member })) => self.members.push(member),
            Some(Ok(ServerMessage::Changed { member })) => replace(&mut self.members, member),
            Some(Ok(ServerMessage::Left { id })) => self.members.retain(|member| member.id != id),
            Some(Ok(_)) => return Err(Error::Protocol("a Welcome, Present or Rejected after the watch began")),
            Some(Err(error)) => return Err(error.into()),
            None => return Err(Error::Protocol("the server ended the watch")),
        }
        Ok(())
    }
}

/// A connection to the server whose first message the server answered.
pub(crate) struct Opened {
    pub(crate) endpoint: Endpoint,
    pub(crate) connection: Connection,
    pub(crate) send: SendStream,
    /// The server's messages after its answer.
    pub(crate) control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
}

impl Opened {
    /// Connects to the server and sends `first`; the server's answer comes along, unless it is
    /// a Rejected or doesn't come within [`JOIN_TIMEOUT`].
    pub(crate) async fn with(server: &str, first: &ClientMessage) -> Result<(Self, ServerMessage), Error> {
        tokio::time::timeout(JOIN_TIMEOUT, Self::unbounded(server, first)).await.unwrap_or(Err(Error::TimedOut))
    }

    async fn unbounded(server: &str, first: &ClientMessage) -> Result<(Self, ServerMessage), Error> {
        let address = tokio::net::lookup_host(server).await?.next().ok_or_else(|| Error::Resolve(server.to_owned()))?;
        let endpoint = Endpoint::client(unspecified_like(address))?;
        let connection = endpoint.connect_with(client_config(), address, SERVER_NAME)?.await?;

        let (mut send, mut recv) = connection.open_bi().await?;
        control::write(&mut send, first).await?;
        let answer = match control::read(&mut recv).await? {
            Some(ServerMessage::Rejected { reason }) => {
                connection.close(0u32.into(), b"rejected");
                return Err(reason.into());
            }
            Some(answer) => answer,
            None => return Err(Error::Protocol("the control stream ended before an answer")),
        };

        let (forward, control) = mpsc::unbounded_channel();
        tokio::spawn(read_control(recv, forward));
        Ok((Self { endpoint, connection, send, control }, answer))
    }
}

/// Sends this member's audio over a session's connection; its clones share the connection.
#[derive(Clone)]
pub struct AudioSender {
    connection: Arc<Mutex<Connection>>,
}

impl AudioSender {
    /// Sends a frame, its `from` left for the server to stamp. A frame the path can't take now is
    /// dropped, as a late one would be.
    pub fn send(&self, frame: &audio::Frame) -> Result<(), quinn::SendDatagramError> {
        self.connection.lock().expect("the connection lock is never poisoned").send_datagram(Bytes::from(frame.encode()))
    }

    /// Sends over the session's connection from now on, as after joining again.
    pub fn follow(&self, session: &Session) {
        *self.connection.lock().expect("the connection lock is never poisoned") = session.connection.clone();
    }
}

/// TLS 1.3 that takes any certificate, and keep-alives that hold the path open.
fn client_config() -> quinn::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = AnyCertificate { algorithms: provider.signature_verification_algorithms };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("the idle timeout fits QUIC's varint")));
    let tls = QuicClientConfig::try_from(tls).expect("the config has TLS 1.3 and no early data");
    let mut client = quinn::ClientConfig::new(Arc::new(tls));
    client.transport_config(Arc::new(transport));
    client
}

/// Takes the certificate the server presents, whichever it is: the tailnet already vouches that
/// the server is the one asked for, and TLS has nothing to add. The handshake's signatures are
/// still checked, as TLS can't go on without.
#[derive(Debug)]
struct AnyCertificate {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Moves the server's control messages into a channel, which `next_event` can wait on beside
/// datagrams: a read cut off mid-message would lose its part.
async fn read_control(mut recv: RecvStream, forward: mpsc::UnboundedSender<Result<ServerMessage, control::Error>>) {
    while let Some(message) = control::read(&mut recv).await.transpose() {
        let failed = message.is_err();
        // A failed send means the session is gone; a failed read ends the stream.
        if forward.send(message).is_err() || failed {
            return;
        }
    }
}

/// Puts the member in the place of the one with its id, if there is one.
pub(crate) fn replace(members: &mut [Member], member: Member) {
    if let Some(old) = members.iter_mut().find(|old| old.id == member.id) {
        *old = member;
    }
}

/// The any-address of the server's family, so the endpoint can reach it.
fn unspecified_like(server: SocketAddr) -> SocketAddr {
    match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    }
}
