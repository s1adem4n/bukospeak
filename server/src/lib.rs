//! The voice server: one channel that relays each member's audio to everyone else.

mod identity;
mod room;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use bukospeak_proto::ALPN;
use quinn::crypto::rustls::QuicServerConfig;

pub use identity::Identity;
pub use room::Room;

/// A connection that stays silent this long is gone; clients send keep-alives well within it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// A QUIC endpoint on `listen` that presents the identity's certificate.
pub fn endpoint(identity: &Identity, listen: SocketAddr) -> anyhow::Result<quinn::Endpoint> {
    let mut tls = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate.clone()], identity.key.clone_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into()?));
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    config.transport_config(Arc::new(transport));

    quinn::Endpoint::server(config, listen).with_context(|| format!("listening on {listen}"))
}
