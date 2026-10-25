//! A terminal client for trying the server: joins the channel on the desktop's audio devices
//! and prints what happens in it.

mod known_servers;

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::sync::{Arc, mpsc};

use anyhow::Context;
use bukospeak_audio_desktop::{Audio, DeviceId, Direction};
use bukospeak_core::proto::MemberId;
use bukospeak_core::{Engine, Error, Event, JoinConfig, Joined, Listener};
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
    /// List the audio devices, the defaults marked with *.
    Devices,
    /// Join the channel and talk, printing who comes, goes and speaks, until Ctrl-C.
    Join {
        /// The server as host:port.
        server: String,
        #[arg(long, env = "BUKOSPEAK_NAME")]
        name: String,
        #[arg(long, env = "BUKOSPEAK_ROOM_KEY", hide_env_values = true)]
        room_key: String,
        /// The microphone's id, as `devices` lists it; the default if not given.
        #[arg(long)]
        input: Option<DeviceId>,
        /// The speakers' or headphones' id; the default if not given.
        #[arg(long)]
        output: Option<DeviceId>,
        /// Send the microphone as captured, for one whose noise is suppressed already.
        #[arg(long)]
        no_noise_suppression: bool,
    },
}

/// What the client waits for.
enum Happening {
    Event(Event),
    AudioFailed(bukospeak_audio_desktop::Error),
    Stop,
}

struct Forward(mpsc::Sender<Happening>);

impl Listener for Forward {
    fn event(&self, event: Event) {
        // The client stops listening only on its way out.
        let _ = self.0.send(Happening::Event(event));
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match Args::parse().command {
        Command::Devices => devices(),
        Command::Join { server, name, room_key, input, output, no_noise_suppression } => {
            join(JoinConfig { server, name, room_key, pin: None }, input, output, !no_noise_suppression)
        }
    }
}

fn devices() -> anyhow::Result<()> {
    for direction in [Direction::Input, Direction::Output] {
        println!("{direction}:");
        for device in bukospeak_audio_desktop::devices(direction)? {
            println!("  {} {}  {}", if device.is_default { "*" } else { " " }, device.id, device.name);
        }
    }
    Ok(())
}

fn join(config: JoinConfig, input: Option<DeviceId>, output: Option<DeviceId>, noise_suppression: bool) -> anyhow::Result<()> {
    let mut known = KnownServers::load()?;
    let server = config.server.clone();
    let pin = known.get(&server);

    let (happenings, received) = mpsc::channel();
    let engine = Engine::new(Arc::new(Forward(happenings.clone())))?;
    engine.set_noise_suppression(noise_suppression);
    let Joined { fingerprint, microphone, speaker } = match engine.join(JoinConfig { pin, ..config }) {
        Err(Error::FingerprintChanged { pinned, presented }) => anyhow::bail!(
            "{server}'s certificate changed from {pinned} to {presented}: if the server was set up anew, remove its line from {}",
            known.path().display()
        ),
        joined => joined.with_context(|| format!("joining {server}"))?,
    };
    if pin.is_none() {
        known.insert(&server, fingerprint)?;
        println!("trusting {server} from now on by its fingerprint {fingerprint}");
    }

    let failures = happenings.clone();
    // The client stops listening only on its way out.
    let audio = Audio::start(microphone, speaker, input, output, move |error| _ = failures.send(Happening::AudioFailed(error)))?;
    ctrlc::set_handler(move || _ = happenings.send(Happening::Stop))?;

    let mut names: BTreeMap<MemberId, String> = BTreeMap::new();
    let mut speaking: Vec<MemberId> = Vec::new();
    loop {
        match received.recv().expect("the client holds a sender until it leaves") {
            Happening::Event(Event::Entered { you, members }) => {
                names = members.into_iter().map(|member| (member.id, member.name)).collect();
                let others: Vec<&str> = names.iter().filter(|&(&id, _)| id != you).map(|(_, name)| name.as_str()).collect();
                println!("joined; also here: {}", if others.is_empty() { "nobody".into() } else { others.join(", ") });
            }
            Happening::Event(Event::Joined(member)) => {
                println!("{} joined", member.name);
                names.insert(member.id, member.name);
            }
            Happening::Event(Event::Left(id)) => {
                if let Some(name) = names.remove(&id) {
                    println!("{name} left");
                }
            }
            Happening::Event(Event::Speaking(now)) => {
                let name = |id: &MemberId| names.get(id).map_or("someone", String::as_str);
                for id in now.iter().filter(|&id| !speaking.contains(id)) {
                    println!("{} speaks", name(id));
                }
                for id in speaking.iter().filter(|&id| !now.contains(id)) {
                    println!("{} is quiet", name(id));
                }
                speaking = now;
            }
            Happening::Event(Event::Disconnected(error)) => return Err(error).context("the call ended"),
            Happening::AudioFailed(error) => eprintln!("{error}"),
            Happening::Stop => break,
        }
    }
    // The devices stop before the call, so nothing plays into a call that is gone.
    drop(audio);
    engine.leave();
    Ok(())
}
