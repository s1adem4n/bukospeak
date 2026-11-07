use std::io::IsTerminal;
use std::sync::{Arc, mpsc};

use anyhow::Context;
use bukospeak_core::proto::MemberId;
use bukospeak_core::{Channel, Conditioning, Engine, Joined, Listener, Update};
use bukospeak_desktop::audio::{self, Audio, DeviceId, Direction};
use bukospeak_desktop::install::Progress;
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
    #[command(about = "List the audio devices, the defaults marked with *")]
    Devices,
    #[command(about = "Join the channel and talk, printing who comes, goes and speaks, until Ctrl-C")]
    Join {
        #[arg(help = "The server as host:port")]
        server: String,
        #[arg(long, help = "The microphone's id, as `devices` lists it; the default if not given")]
        input: Option<DeviceId>,
        #[arg(long, help = "The speakers' or headphones' id; the default if not given")]
        output: Option<DeviceId>,
        #[arg(
            long,
            value_enum,
            default_value_t = Suppression::Standard,
            help = "How the microphone's noise is suppressed; NVIDIA's effects are installed from the server first"
        )]
        suppression: Suppression,
        #[arg(long, help = "Leave the speakers' echo in what the microphone hears, for a headset")]
        no_echo_cancellation: bool,
    },
}

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
        Command::Join { server, input, output, suppression, no_echo_cancellation } => {
            let processing = Processing { suppression, echo_cancellation: !no_echo_cancellation };
            join(&server, input, output, processing)
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

struct Processing {
    suppression: Suppression,
    echo_cancellation: bool,
}

fn join(server: &str, input: Option<DeviceId>, output: Option<DeviceId>, processing: Processing) -> anyhow::Result<()> {
    let effects = match processing.suppression.nvidia() {
        Some(_) => Some(install(server)?),
        None => None,
    };

    let (happenings, received) = mpsc::channel();
    // Desktop devices pass the microphone's signal unprocessed.
    let engine = Engine::new(Conditioning::Engine, Arc::new(Forward(happenings.clone())))?;
    engine.set_suppression(processing.suppression.engines(effects.as_ref()))?;
    engine.set_echo_cancellation(processing.echo_cancellation);
    let Joined { microphone, speaker } = engine.join(server).with_context(|| format!("joining {server}"))?;

    let failures = happenings.clone();
    // The client stops listening only on its way out.
    let audio = Audio::call(microphone, speaker, None, input, output, move |error| _ = failures.send(Happening::AudioFailed(error)))?;
    ctrlc::set_handler(move || _ = happenings.send(Happening::Stop))?;

    let mut printed = Channel::default();
    loop {
        match received.recv().expect("the client holds a sender until it leaves") {
            Happening::Update(update) => {
                let Update { channel, announcement, lost } = *update;
                if let Some(lost) = lost {
                    return Err(lost).context("the call ended");
                }
                if printed.you().is_none()
                    && let Some(you) = channel.you()
                {
                    let others: Vec<&str> =
                        channel.members().iter().filter(|member| member.id != you).map(|member| member.name.as_str()).collect();
                    println!("joined; also here: {}", if others.is_empty() { "nobody".into() } else { others.join(", ") });
                }
                if let Some(announcement) = announcement {
                    println!("{announcement}");
                }
                for &id in channel.speaking().iter().filter(|id| !printed.speaking().contains(id)) {
                    println!("{} speaks", name(&channel, id));
                }
                for &id in printed.speaking().iter().filter(|id| !channel.speaking().contains(id)) {
                    println!("{} is quiet", name(&printed, id));
                }
                printed = channel;
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

fn install(server: &str) -> anyhow::Result<Arc<Effects>> {
    let architecture = nvidia::card().context("NVIDIA's effects need an RTX card under Windows")?;
    let mut shown = None;
    nvidia::install(server, architecture, |progress| {
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

fn name(channel: &Channel, id: MemberId) -> &str {
    channel.members().iter().find(|member| member.id == id).map_or("someone", |member| &member.name)
}
