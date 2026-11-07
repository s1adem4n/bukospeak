use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use bukospeak_server::{Downloads, LocalApi, Ngc, Room, Sounds, Source, endpoint};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(long, env = "BUKOSPEAK_LISTEN", default_value = "[::]:9988", help = "The UDP address to listen on")]
    listen: SocketAddr,
    #[arg(long, env = "BUKOSPEAK_DATA", default_value = "data", help = "The folder of the downloads and, in `sounds`, the call's sounds")]
    data: PathBuf,
    #[arg(
        long,
        env = "BUKOSPEAK_TAILSCALED",
        default_value = LocalApi::DEFAULT_SOCKET,
        help = "The LocalAPI socket of the tailscaled beside the server; whoever is on its tailnet may join"
    )]
    tailscaled: PathBuf,
    #[arg(long, env = "BUKOSPEAK_NGC_KEY", hide_env_values = true, help = "An NGC API key, with which the server offers NVIDIA's effects")]
    ngc_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let args = Args::parse();

    let endpoint = endpoint(args.listen)?;
    tracing::info!(listen = %args.listen, tailscaled = %args.tailscaled.display(), "serving");

    let mut sources: Vec<Arc<dyn Source>> = vec![Arc::new(Sounds::new(args.data.join("sounds")))];
    sources.extend(args.ngc_key.map(|key| Arc::new(Ngc::new(key)) as Arc<dyn Source>));
    let downloads = Downloads::new(args.data.join("downloads"), sources);
    tokio::select! {
        () = Room::new(LocalApi::new(args.tailscaled), downloads).serve(endpoint.clone()) => {}
        () = shutdown() => {}
    }
    endpoint.close(0u32.into(), b"server stopped");
    endpoint.wait_idle().await;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        // A container stop sends SIGTERM.
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).expect("installing the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.expect("installing the Ctrl-C handler");
}
