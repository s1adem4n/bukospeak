//! The call as the desktop app has it: joins, leaves and watches the channel on a worker thread
//! of its own, plays the call through a [`Player`], and keeps what the window shows of it.
//!
//! Joins and leaves block on the server, so they run on the worker, one after the other. What
//! comes back, from the worker and from the engine, arrives as [`Happening`]s on one channel,
//! which the app hands to [`Call::happen`] on its own thread, in order.
//!
//! Each of the engine's updates tells which of its joins it is of, and its leave returns the
//! last: the worker hands that back in order with the updates, and updates of a call left, still
//! on their way, are dropped.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use bukospeak_audio_desktop::{Audio, DeviceId, Direction};
use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::proto::{Fingerprint, MemberId};
use bukospeak_core::{Conditioning, Engine, Error, JoinConfig, Joined, Listener, Microphone, Speaker, State, Update};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::known_servers::KnownServers;

/// Where the call is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Joining,
    Connected,
    /// The connection was lost, and the engine joins again.
    Reconnecting,
    Failed(Failure),
}

/// Why a join failed or a call ended without a leave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server presented another certificate than the one pinned: it was set up anew, or
    /// someone pretends to be it.
    CertificateChanged {
        server: String,
        pinned: Fingerprint,
        presented: Fingerprint,
    },
    Rejected(Rejection),
    /// The server's name doesn't resolve.
    NotFound,
    /// The server didn't answer the join in time.
    NoAnswer,
    /// No connection to the server came about, or it broke during the join.
    Unreachable,
    /// The fingerprints of servers joined before couldn't be read.
    KnownServers,
    /// The microphone and speakers didn't start.
    Audio,
    /// The connection was lost, and joining again failed.
    ConnectionLost,
}

/// Something the user should know beside the call, which goes on without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A device isn't there, or no device is.
    DeviceMissing(Direction),
    /// A device is there but doesn't play.
    DeviceFailed(Direction),
    /// The server's fingerprint wasn't kept: the next join trusts it on first use again, or,
    /// for a changed certificate, asks again.
    PinNotKept,
}

/// The devices a call plays on; the system's defaults for None.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub input: Option<DeviceId>,
    pub output: Option<DeviceId>,
}

/// What a call needs to join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    /// The server as `host:port`.
    pub server: String,
    pub name: String,
    pub room_key: String,
    pub devices: Devices,
}

/// Hears why a device doesn't play, once each time it stops.
pub type Report = Box<dyn Fn(bukospeak_audio_desktop::Error) + Send>;

/// Plays a call's two ends: on the desktop's devices, or on stand-ins in tests.
pub trait Player: Send + 'static {
    fn play(&mut self, microphone: Microphone, speaker: Speaker, devices: &Devices, report: Report) -> io::Result<Box<dyn Playing>>;
}

/// A call playing, until it is dropped.
pub trait Playing: Send {
    fn set_device(&self, direction: Direction, device: Option<DeviceId>);
}

/// Plays calls on the desktop's devices.
pub struct DesktopPlayer;

impl Player for DesktopPlayer {
    fn play(&mut self, microphone: Microphone, speaker: Speaker, devices: &Devices, report: Report) -> io::Result<Box<dyn Playing>> {
        let audio = Audio::start(microphone, speaker, devices.input.clone(), devices.output.clone(), report)?;
        Ok(Box::new(audio))
    }
}

impl Playing for Audio {
    fn set_device(&self, direction: Direction, device: Option<DeviceId>) {
        match direction {
            Direction::Input => self.set_input(device),
            Direction::Output => self.set_output(device),
        }
    }
}

/// Something that came back from the worker or the engine, for [`Call::happen`].
pub struct Happening(Event);

enum Event {
    Update(Box<Update>),
    /// A join came back; the number is the one [`Call`] gave it.
    Joined(u64, Result<(), Failure>),
    /// A leave came back with the engine's call it left.
    Left(u64),
    /// A call's device report, by the call's number.
    Device(u64, bukospeak_audio_desktop::Error),
    Notice(Notice),
}

enum Command {
    Join { number: u64, join: Join, trust: Option<Fingerprint> },
    Leave,
    Watch { server: String, room_key: String },
    Unwatch,
    SetDevice(Direction, Option<DeviceId>),
}

/// The call: joins and leaves the channel, and keeps what the window shows of it.
pub struct Call {
    engine: Arc<Engine>,
    commands: Option<mpsc::Sender<Command>>,
    worker: Option<thread::JoinHandle<()>>,
    /// The number of the call the worker should play; 0 for none. A join that comes back with
    /// another doesn't play, and the leave after it ends it.
    current: Arc<AtomicU64>,
    numbers: u64,
    /// The join under way or the call up, by its number, and its server.
    call: Option<(u64, String)>,
    /// Whether the call's join came back.
    joined: bool,
    /// Leaves sent whose engine call hasn't come back: updates meanwhile may be of the call left.
    leaves: usize,
    /// The engine's call left last: updates of it or of one before are stale.
    left_call: u64,
    /// The server and room key to watch while out of a call, if any.
    watched: Option<(String, String)>,
    /// The engine's state of the call, which may join again before its join came back here.
    engine_state: State,
    phase: Phase,
    you: Option<MemberId>,
    members: Vec<Member>,
    speaking: Vec<MemberId>,
    present: Vec<Member>,
    muted: bool,
    notice: Option<Notice>,
}

impl Call {
    /// A call with no one in it yet, which pins servers in `known` and plays through `player`;
    /// what happens comes on the receiver.
    pub fn new(known: KnownServers, player: impl Player) -> io::Result<(Self, UnboundedReceiver<Happening>)> {
        let (happenings, received) = unbounded();
        // The desktop's audio passes the microphone's signal as it is.
        let engine = Arc::new(Engine::new(Conditioning::Engine, Arc::new(Forward(happenings.clone())))?);
        let (commands, inbox) = mpsc::channel();
        let current = Arc::new(AtomicU64::new(0));
        let worker = Worker { engine: Arc::clone(&engine), known, player: Box::new(player), happenings, current: Arc::clone(&current) };
        let worker = thread::Builder::new().name("bukospeak-call".into()).spawn(move || worker.run(&inbox))?;
        let call = Self {
            engine,
            commands: Some(commands),
            worker: Some(worker),
            current,
            numbers: 0,
            call: None,
            joined: false,
            leaves: 0,
            left_call: 0,
            watched: None,
            engine_state: State::Out,
            phase: Phase::Idle,
            you: None,
            members: Vec::new(),
            speaking: Vec::new(),
            present: Vec::new(),
            muted: false,
            notice: None,
        };
        Ok((call, received))
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Whether a call is up, joining again included.
    pub fn is_in_call(&self) -> bool {
        matches!(self.phase, Phase::Connected | Phase::Reconnecting)
    }

    /// This member's id, while in the call.
    pub fn you(&self) -> Option<MemberId> {
        self.you
    }

    /// Everyone in the call, this member included, in the order they came.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Who speaks now, this member among them.
    pub fn speaking(&self) -> &[MemberId] {
        &self.speaking
    }

    /// Who is in the channel, watched from outside it.
    pub fn present(&self) -> &[Member] {
        &self.present
    }

    /// Whether this member is muted: the microphone sends silence and the others see it muted.
    /// It holds across calls.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Joins the channel, trusting the server by its pinned fingerprint, or on first use if none
    /// is; the worker leaves the call before first.
    pub fn join(&mut self, join: Join) {
        self.start(join, None);
    }

    /// Joins again after the server's certificate changed, trusting exactly the one it presented.
    pub fn trust(&mut self, join: Join) {
        if let Phase::Failed(Failure::CertificateChanged { server, presented, .. }) = &self.phase
            && *server == join.server
        {
            let presented = *presented;
            self.start(join, Some(presented));
        }
    }

    /// Leaves the call, or the join under way.
    pub fn leave(&mut self) {
        self.end(None);
    }

    /// Forgets the failure and the notice the user has seen.
    pub fn dismiss(&mut self) {
        if matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Idle;
        }
        self.notice = None;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.engine.set_muted(muted);
    }

    pub fn set_noise_suppression(&self, on: bool) {
        self.engine.set_noise_suppression(on);
    }

    pub fn set_echo_cancellation(&self, on: bool) {
        self.engine.set_echo_cancellation(on);
    }

    /// Plays the call on another device from now on; the next join is told its devices anew.
    pub fn set_device(&self, direction: Direction, device: Option<DeviceId>) {
        self.send(Command::SetDevice(direction, device));
    }

    /// Watches who is in the channel on a server joined before, while out of a call; for None,
    /// nowhere.
    pub fn watch(&mut self, channel: Option<(String, String)>) {
        if self.watched != channel {
            self.watched = channel;
            self.rewatch();
        }
    }

    /// Takes in what came back.
    pub fn happen(&mut self, happening: Happening) {
        match happening.0 {
            Event::Update(update) => self.update(*update),
            Event::Joined(number, result) => self.joined(number, result),
            Event::Left(call) => {
                self.leaves -= 1;
                self.left_call = call;
            }
            Event::Device(number, error) => {
                tracing::warn!("{error}");
                if self.number() == Some(number) {
                    self.notice = Some(notice(&error));
                }
            }
            Event::Notice(notice) => self.notice = Some(notice),
        }
    }

    fn number(&self) -> Option<u64> {
        self.call.as_ref().map(|(number, _)| *number)
    }

    fn start(&mut self, join: Join, trust: Option<Fingerprint>) {
        if self.call.is_some() {
            return;
        }
        self.numbers += 1;
        let number = self.numbers;
        self.call = Some((number, join.server.clone()));
        (self.joined, self.engine_state) = (false, State::Out);
        self.phase = Phase::Joining;
        self.notice = None;
        self.current.store(number, Ordering::Relaxed);
        // The engine stops watching on a join.
        self.send(Command::Join { number, join, trust });
    }

    fn joined(&mut self, number: u64, result: Result<(), Failure>) {
        if self.number() != Some(number) {
            return;
        }
        match result {
            Ok(()) => {
                self.joined = true;
                self.follow();
            }
            // The engine has a call for the audio that didn't start; the leave ends it.
            Err(Failure::Audio) => self.end(Some(Failure::Audio)),
            Err(failure) => {
                self.call = None;
                self.current.store(0, Ordering::Relaxed);
                self.phase = Phase::Failed(failure);
                self.rewatch();
            }
        }
    }

    fn update(&mut self, update: Update) {
        let Update { channel, lost, .. } = update;
        self.present = channel.present().to_vec();
        // Out of a call, or of one left whose last updates still arrive.
        if self.call.is_none() || self.leaves > 0 || channel.call() <= self.left_call {
            return;
        }
        self.engine_state = channel.state();
        self.you = channel.you();
        self.members = channel.members().to_vec();
        self.speaking = channel.speaking().to_vec();
        if let Some(lost) = lost {
            let failure = match (lost.rejoin, &self.call) {
                (Error::FingerprintChanged { pinned, presented }, Some((_, server))) => {
                    Failure::CertificateChanged { server: server.clone(), pinned, presented }
                }
                _ => Failure::ConnectionLost,
            };
            return self.end(Some(failure));
        }
        self.follow();
    }

    /// Shows the call as the engine has it, once its join came back: up only once the engine
    /// entered the channel too, so a call shows connected with who is in it.
    fn follow(&mut self) {
        if !self.joined {
            return;
        }
        match self.engine_state {
            State::Connected => self.phase = Phase::Connected,
            State::Reconnecting => self.phase = Phase::Reconnecting,
            // Not entered yet: the update that tells who is in the call is still on its way. A
            // call left or lost ended here before its last update.
            State::Out => {}
        }
    }

    /// Ends the call on the screen at once, with why if it failed; the worker stops its audio
    /// and leaves.
    fn end(&mut self, failure: Option<Failure>) {
        if self.call.take().is_none() {
            return;
        }
        self.current.store(0, Ordering::Relaxed);
        self.engine_state = State::Out;
        self.you = None;
        self.members.clear();
        self.speaking.clear();
        self.phase = failure.map_or(Phase::Idle, Phase::Failed);
        self.leaves += 1;
        self.send(Command::Leave);
        self.rewatch();
    }

    /// Watches what is to be watched, while out of a call; the engine stops watching on a join.
    fn rewatch(&mut self) {
        if self.call.is_some() {
            return;
        }
        let command = match &self.watched {
            Some((server, room_key)) => Command::Watch { server: server.clone(), room_key: room_key.clone() },
            None => Command::Unwatch,
        };
        self.send(command);
    }

    fn send(&self, command: Command) {
        let commands = self.commands.as_ref().expect("the commands close only on drop");
        // The worker runs until the commands close.
        commands.send(command).expect("the worker runs while the call is there");
    }
}

impl Drop for Call {
    /// Stops the call's audio and leaves the call: a join under way finishes first, so this
    /// may wait for the server.
    fn drop(&mut self) {
        self.current.store(0, Ordering::Relaxed);
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            worker.join().expect("the worker doesn't panic");
        }
    }
}

/// What the user is told of a device's report.
fn notice(error: &bukospeak_audio_desktop::Error) -> Notice {
    use bukospeak_audio_desktop::Error;
    match *error {
        Error::Missing(direction, _) | Error::NoDevice(direction) => Notice::DeviceMissing(direction),
        Error::NoFormat(direction) | Error::Format(direction, _) | Error::Device(direction, _) => Notice::DeviceFailed(direction),
    }
}

/// What the user is told of a failed join to `server`.
fn failure(error: Error, server: String) -> Failure {
    match error {
        Error::FingerprintChanged { pinned, presented } => Failure::CertificateChanged { server, pinned, presented },
        Error::Rejected(rejection) => Failure::Rejected(rejection),
        Error::Resolve(_) => Failure::NotFound,
        Error::TimedOut => Failure::NoAnswer,
        Error::Connect(_) | Error::Connection(_) | Error::Control(_) | Error::Protocol(_) | Error::Io(_) => Failure::Unreachable,
    }
}

/// Hands the engine's updates on, without waiting.
struct Forward(UnboundedSender<Happening>);

impl Listener for Forward {
    fn update(&self, update: Update) {
        // Closed only when the call is dropped, which stops the engine.
        let _ = self.0.unbounded_send(Happening(Event::Update(Box::new(update))));
    }
}

/// What the worker thread holds: it joins, leaves and watches, and holds the call's audio.
struct Worker {
    engine: Arc<Engine>,
    known: KnownServers,
    player: Box<dyn Player>,
    happenings: UnboundedSender<Happening>,
    current: Arc<AtomicU64>,
}

impl Worker {
    fn run(mut self, inbox: &mpsc::Receiver<Command>) {
        let mut playing: Option<Box<dyn Playing>> = None;
        for command in inbox {
            match command {
                Command::Join { number, join, trust } => playing = self.join(number, join, trust),
                Command::Leave => {
                    // The audio stops before the call, so nothing plays into a call that is gone.
                    playing = None;
                    let left = self.engine.leave();
                    self.tell(Event::Left(left));
                }
                Command::Watch { server, room_key } => match self.known.get(&server) {
                    Ok(Some(pin)) => self.engine.watch(server, room_key, pin),
                    // A server never joined isn't trusted yet.
                    Ok(None) => self.engine.unwatch(),
                    // Who is in the channel is a glance before joining; a join tells the user.
                    Err(error) => {
                        tracing::warn!("{error}");
                        self.engine.unwatch();
                    }
                },
                Command::Unwatch => self.engine.unwatch(),
                Command::SetDevice(direction, device) => {
                    if let Some(playing) = &playing {
                        playing.set_device(direction, device);
                    }
                }
            }
        }
        drop(playing);
        self.engine.leave();
    }

    /// Joins, pins the server and starts the call's audio, and tells the call how it went; the
    /// audio returned plays the call until the next leave.
    fn join(&mut self, number: u64, join: Join, trust: Option<Fingerprint>) -> Option<Box<dyn Playing>> {
        let Join { server, name, room_key, devices } = join;
        let pinned = match self.known.get(&server) {
            Ok(pinned) => pinned,
            Err(error) => {
                tracing::warn!("{error}");
                return self.joined(number, Err(Failure::KnownServers));
            }
        };
        let config = JoinConfig { server: server.clone(), name, room_key, pin: trust.or(pinned) };
        let Joined { fingerprint, microphone, speaker } = match self.engine.join(config) {
            Ok(joined) => joined,
            Err(error) => {
                tracing::warn!("joining {server}: {error}");
                return self.joined(number, Err(failure(error, server)));
            }
        };
        if self.current.load(Ordering::Relaxed) != number {
            // Left while joining: the leave after this join ends the call before it plays.
            return None;
        }
        if pinned != Some(fingerprint)
            && let Err(error) = self.known.insert(&server, fingerprint)
        {
            tracing::warn!("{error}");
            self.tell(Event::Notice(Notice::PinNotKept));
        }
        let happenings = self.happenings.clone();
        let report: Report = Box::new(move |error| {
            // Closed only when the call is dropped, which stops the audio.
            let _ = happenings.unbounded_send(Happening(Event::Device(number, error)));
        });
        match self.player.play(microphone, speaker, &devices, report) {
            Ok(playing) => {
                self.tell(Event::Joined(number, Ok(())));
                Some(playing)
            }
            Err(error) => {
                tracing::warn!("the audio didn't start: {error}");
                // The call leaves on this failure, which ends the engine's call.
                self.joined(number, Err(Failure::Audio))
            }
        }
    }

    /// Tells the call how its join went; nothing plays.
    fn joined(&self, number: u64, result: Result<(), Failure>) -> Option<Box<dyn Playing>> {
        self.tell(Event::Joined(number, result));
        None
    }

    fn tell(&self, event: Event) {
        // Closed only when the call is dropped, which ends the worker after this command.
        let _ = self.happenings.unbounded_send(Happening(event));
    }
}
