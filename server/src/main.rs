use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;

use bukospeak_server::{Identity, Room, endpoint};
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// The BukoSpeak voice server.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The UDP address to listen on.
    #[arg(long, env = "BUKOSPEAK_LISTEN", default_value = "[::]:9988")]
    listen: SocketAddr,
    /// Where the server keeps its certificate.
    #[arg(long, env = "BUKOSPEAK_DATA", default_value = "data")]
    data: PathBuf,
    /// The key members need to join.
    #[arg(long, env = "BUKOSPEAK_ROOM_KEY", hide_env_values = true)]
    room_key: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let args = Args::parse();

    let identity = Identity::load_or_create(&args.data)?;
    let endpoint = endpoint(&identity, args.listen)?;
    tracing::info!(listen = %args.listen, fingerprint = %identity.fingerprint(), "serving");

    tokio::select! {
        () = Room::new(args.room_key).serve(endpoint.clone()) => {}
        () = shutdown() => {}
    }
    endpoint.close(0u32.into(), b"server stopped");
    endpoint.wait_idle().await;
    Ok(())
}

/// Ctrl-C, or the SIGTERM a container stop sends.
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("installing the SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}
