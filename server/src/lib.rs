mod certificate;
mod downloads;
mod ngc;
mod room;
mod sounds;
mod tailnet;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use bukospeak_proto::{ALPN, IDLE_TIMEOUT};
use quinn::crypto::rustls::QuicServerConfig;

pub use downloads::{Downloads, Source, compress};
pub use ngc::Ngc;
pub use room::Room;
pub use sounds::Sounds;
pub use tailnet::{Device, LocalApi, Tailnet};

pub fn endpoint(listen: SocketAddr) -> anyhow::Result<quinn::Endpoint> {
    let (certificate, key) = certificate::generate()?;
    let mut tls = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into()?));
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    config.transport_config(Arc::new(transport));

    quinn::Endpoint::server(config, listen).with_context(|| format!("listening on {listen}"))
}
