//! A terminal client for trying the server: joins the channel on the desktop's audio devices
//! and prints what happens in it.

use std::io::IsTerminal;
use std::sync::{Arc, mpsc};

use anyhow::Context;
use bukospeak_core::proto::MemberId;
use bukospeak_core::{Channel, Conditioning, Engine, Error, JoinConfig, Joined, Listener, Update};
use bukospeak_desktop::audio::{self, Audio, DeviceId, Direction};
use bukospeak_desktop::install::Progress;
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_desktop::nvidia::{self, Effects};
use bukospeak_desktop::settings::Suppression;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "A terminal client for trying the BukoSpeak server")]
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
        /// How the microphone's noise is suppressed: NVIDIA's effects are installed from the
        /// server first, if they aren't yet.
        #[arg(long, value_enum, default_value_t = Suppression::Standard)]
        suppression: Suppression,
        /// Leave the speakers' echo in what the microphone hears, for a headset.
        #[arg(long)]
        no_echo_cancellation: bool,
    },
}

/// What the client waits for.
enum Happening {
    Update(Box<Update>),
    AudioFailed(audio::Error),
    Stop,
}

struct Forward(mpsc::Sender<Happening>);

impl Listener for Forward {
    fn update(&self, update: Update) {
        // The client stops listening only on its way out.
        let _ = self.0.send(Happening::Update(Box::new(update)));
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| bukospeak_desktop::LOG_FILTER.into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match Args::parse().command {
        Command::Devices => devices(),
        Command::Join { server, name, room_key, input, output, suppression, no_echo_cancellation } => {
            let processing = Processing { suppression, echo_cancellation: !no_echo_cancellation };
            join(JoinConfig { server, name, room_key, pin: None }, input, output, processing)
        }
    }
}

fn devices() -> anyhow::Result<()> {
    for direction in [Direction::Input, Direction::Output] {
        println!("{direction}:");
        for device in audio::devices(direction)? {
            println!("  {} {}  {}", if device.is_default { "*" } else { " " }, device.id, device.name);
        }
    }
    Ok(())
}

/// What the engine does to the microphone's signal.
struct Processing {
    suppression: Suppression,
    echo_cancellation: bool,
}

fn join(config: JoinConfig, input: Option<DeviceId>, output: Option<DeviceId>, processing: Processing) -> anyhow::Result<()> {
    let folder = bukospeak_desktop::config_folder().context("this system has no config folder")?;
    let known = KnownServers::at(folder.join("known_servers"));
    let server = config.server.clone();
    let pin = known.get(&server)?;

    let effects = match processing.suppression.nvidia() {
        Some(_) => Some(install(&config, &known)?),
        None => None,
    };

    let (happenings, received) = mpsc::channel();
    // `cpal` passes the microphone's signal as it is.
    let engine = Engine::new(Conditioning::Engine, Arc::new(Forward(happenings.clone())))?;
    engine.set_suppression(processing.suppression.engines(effects.as_ref()))?;
    engine.set_echo_cancellation(processing.echo_cancellation);
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
    let audio = Audio::call(microphone, speaker, None, input, output, move |error| _ = failures.send(Happening::AudioFailed(error)))?;
    ctrlc::set_handler(move || _ = happenings.send(Happening::Stop))?;

    // The channel as last printed.
    let mut shown = Channel::default();
    loop {
        match received.recv().expect("the client holds a sender until it leaves") {
            Happening::Update(update) => {
                let Update { channel, announcement, lost } = *update;
                if let Some(lost) = lost {
                    return Err(lost).context("the call ended");
                }
                if shown.you().is_none()
                    && let Some(you) = channel.you()
                {
                    let others: Vec<&str> =
                        channel.members().iter().filter(|member| member.id != you).map(|member| member.name.as_str()).collect();
                    println!("joined; also here: {}", if others.is_empty() { "nobody".into() } else { others.join(", ") });
                }
                if let Some(announcement) = announcement {
                    println!("{announcement}");
                }
                for &id in channel.speaking().iter().filter(|id| !shown.speaking().contains(id)) {
                    println!("{} speaks", name(&channel, id));
                }
                for &id in shown.speaking().iter().filter(|id| !channel.speaking().contains(id)) {
                    println!("{} is quiet", name(&shown, id));
                }
                shown = channel;
            }
            Happening::AudioFailed(error) => eprintln!("{error}"),
            Happening::Stop => break,
        }
    }
    // The devices stop before the call, so nothing plays into a call that is gone.
    drop(audio);
    engine.leave();
    Ok(())
}

/// Installs NVIDIA's effects for the system's card from the server, unless they are installed,
/// telling how far it got.
fn install(config: &JoinConfig, known: &KnownServers) -> anyhow::Result<Arc<Effects>> {
    let architecture = nvidia::card().context("NVIDIA's effects need an RTX card under Windows")?;
    let mut shown = None;
    nvidia::install(&config.server, config.room_key.clone(), known, architecture, |progress| {
        let (what, done, total) = match progress {
            Progress::Preparing { done, total } => ("the server gets NVIDIA's effects", done, total),
            Progress::Fetching { done, total } => ("downloading NVIDIA's effects", done, total),
        };
        let percent = (done * 100).checked_div(total).unwrap_or(0);
        if shown.replace((what, percent)) != Some((what, percent)) {
            eprint!("\r{what}: {percent} %   ");
        }
    })
    .with_context(|| format!("installing NVIDIA's effects for {}", architecture.name()))?;
    if shown.is_some() {
        eprintln!();
    }
    let failed = || eprintln!("NVIDIA's effect didn't start: the built-in suppression runs instead");
    Effects::installed(architecture, failed).context("NVIDIA's effects aren't where they were installed")
}

/// A member's name, as the channel knows it.
fn name(channel: &Channel, id: MemberId) -> &str {
    channel.members().iter().find(|member| member.id == id).map_or("someone", |member| &member.name)
}
