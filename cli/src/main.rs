//! A terminal client for trying the server: joins the channel and prints what happens in it.

mod known_servers;

use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;

use anyhow::Context;
use bukospeak_core::proto::MemberId;
use bukospeak_core::{Error, Event, JoinConfig, Session};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::known_servers::KnownServers;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Join the channel and print who comes, goes and speaks, until Ctrl-C.
    Join {
        /// The server as host:port.
        server: String,
        #[arg(long, env = "BUKOSPEAK_NAME")]
        name: String,
        #[arg(long, env = "BUKOSPEAK_ROOM_KEY", hide_env_values = true)]
        room_key: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match Args::parse().command {
        Command::Join { server, name, room_key } => join(server, name, room_key).await,
    }
}

async fn join(server: String, name: String, room_key: String) -> anyhow::Result<()> {
    let mut known = KnownServers::load()?;
    let pin = known.get(&server);
    let mut session = match Session::join(JoinConfig { server: server.clone(), name, room_key, pin }).await {
        Err(Error::FingerprintChanged { pinned, presented }) => anyhow::bail!(
            "{server}'s certificate changed from {pinned} to {presented}: if the server was set up anew, remove its line from {}",
            known.path().display()
        ),
        joined => joined.with_context(|| format!("joining {server}"))?,
    };
    if pin.is_none() {
        known.insert(&server, session.fingerprint())?;
        println!("trusting {server} from now on by its fingerprint {}", session.fingerprint());
    }

    let mut names: BTreeMap<MemberId, String> = session.members().iter().map(|member| (member.id, member.name.clone())).collect();
    let others: Vec<&str> = names.iter().filter(|&(&id, _)| id != session.you()).map(|(_, name)| name.as_str()).collect();
    println!("joined; also here: {}", if others.is_empty() { "nobody".into() } else { others.join(", ") });

    let mut speaking = BTreeSet::new();
    loop {
        let event = tokio::select! {
            event = session.next_event() => event?,
            _ = tokio::signal::ctrl_c() => break,
        };
        match event {
            Event::Joined(member) => {
                println!("{} joined", member.name);
                names.insert(member.id, member.name);
            }
            Event::Left(id) => {
                speaking.remove(&id);
                println!("{} left", names.remove(&id).unwrap_or_default());
            }
            Event::Audio(frame) => {
                let name = names.get(&frame.from).map_or("someone", String::as_str);
                if frame.last {
                    speaking.remove(&frame.from);
                    println!("{name} is quiet");
                } else if speaking.insert(frame.from) {
                    println!("{name} speaks");
                }
            }
        }
    }
    session.leave().await;
    Ok(())
}
