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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Joined(Member),
    Changed(Member),
    Left(MemberId),
    Audio(audio::Frame),
}

pub struct Session {
    endpoint: Endpoint,
    connection: Connection,
    // Finishing it tells the server the member left.
    send: SendStream,
    control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
    you: MemberId,
    members: Vec<Member>,
}

impl Session {
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

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn audio_sender(&self) -> AudioSender {
        AudioSender { connection: Arc::new(Mutex::new(self.connection.clone())) }
    }

    pub fn received(&self) -> u64 {
        self.connection.stats().udp_rx.datagrams
    }

    pub async fn set_muted(&mut self, muted: bool) -> Result<(), Error> {
        Ok(control::write(&mut self.send, &ClientMessage::Mute { muted }).await?)
    }

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

pub struct Watch {
    // Dropping them closes the connection, which ends the watch at the server.
    _endpoint: Endpoint,
    _connection: Connection,
    _send: SendStream,
    control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
    members: Vec<Member>,
}

impl Watch {
    pub async fn start(server: &str) -> Result<Self, Error> {
        let (opened, answer) = Opened::with(server, &ClientMessage::Watch { version: PROTOCOL_VERSION }).await?;
        let ServerMessage::Present { members } = answer else {
            return Err(Error::Protocol("the first message isn't a Present"));
        };
        let Opened { endpoint, connection, send, control, .. } = opened;
        Ok(Self { _endpoint: endpoint, _connection: connection, _send: send, control, members })
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

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

pub(crate) struct Opened {
    pub(crate) endpoint: Endpoint,
    pub(crate) connection: Connection,
    pub(crate) send: SendStream,
    pub(crate) control: mpsc::UnboundedReceiver<Result<ServerMessage, control::Error>>,
}

impl Opened {
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

#[derive(Clone)]
pub struct AudioSender {
    connection: Arc<Mutex<Connection>>,
}

impl AudioSender {
    pub fn send(&self, frame: &audio::Frame) -> Result<(), quinn::SendDatagramError> {
        self.connection.lock().expect("the connection lock is never poisoned").send_datagram(Bytes::from(frame.encode()))
    }

    pub fn follow(&self, session: &Session) {
        *self.connection.lock().expect("the connection lock is never poisoned") = session.connection.clone();
    }
}

// The tailnet vouches for the server; TLS, which QUIC can't do without, has nothing to add.
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

// A read cut off mid-message by select! would lose its part, so a task of its own reads.
async fn read_control(mut recv: RecvStream, forward: mpsc::UnboundedSender<Result<ServerMessage, control::Error>>) {
    while let Some(message) = control::read(&mut recv).await.transpose() {
        let failed = message.is_err();
        if forward.send(message).is_err() || failed {
            return;
        }
    }
}

pub(crate) fn replace(members: &mut [Member], member: Member) {
    if let Some(old) = members.iter_mut().find(|old| old.id == member.id) {
        *old = member;
    }
}

fn unspecified_like(server: SocketAddr) -> SocketAddr {
    match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    }
}
