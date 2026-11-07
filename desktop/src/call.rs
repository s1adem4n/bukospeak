use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use crate::audio::{self, Audio, DeviceId, Direction};
pub use bukospeak_core::Failure;
use bukospeak_core::proto::MemberId;
use bukospeak_core::proto::control::Member;
use bukospeak_core::{Channel, Conditioning, Engine, Joined, Listener, Meter, Microphone, Speaker, State, Suppression, Update};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Joining,
    Connected,
    Reconnecting,
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    DeviceMissing(Direction),
    DeviceFailed(Direction),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub input: Option<DeviceId>,
    pub output: Option<DeviceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub server: String,
    pub devices: Devices,
}

pub type Report = Box<dyn Fn(audio::Error) + Send>;

pub trait Player: Send + 'static {
    fn play(
        &mut self,
        microphone: Microphone,
        speaker: Speaker,
        meter: &Meter,
        devices: &Devices,
        report: Report,
    ) -> io::Result<Box<dyn Playing>>;

    fn listen(&mut self, meter: &Meter, input: Option<&DeviceId>, report: Report) -> io::Result<Box<dyn Playing>>;
}

pub trait Playing: Send {
    fn set_device(&self, direction: Direction, device: Option<DeviceId>);
}

pub struct DesktopPlayer;

impl Player for DesktopPlayer {
    fn play(
        &mut self,
        microphone: Microphone,
        speaker: Speaker,
        meter: &Meter,
        devices: &Devices,
        report: Report,
    ) -> io::Result<Box<dyn Playing>> {
        let audio = Audio::call(microphone, speaker, Some(meter.clone()), devices.input.clone(), devices.output.clone(), report)?;
        Ok(Box::new(audio))
    }

    fn listen(&mut self, meter: &Meter, input: Option<&DeviceId>, report: Report) -> io::Result<Box<dyn Playing>> {
        Ok(Box::new(Audio::listen(meter.clone(), input.cloned(), report)?))
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

pub struct Happening(Event);

enum Event {
    Update(Box<Update>),
    Joined(u64, Result<(), Failure>),
    Left(u64),
    Device(u64, audio::Error),
}

enum Command {
    Join { number: u64, join: Join, asked: Instant },
    Leave,
    Watch(String),
    Unwatch,
    Listen(bool),
    SetDevice(Direction, Option<DeviceId>),
}

pub struct Call {
    engine: Arc<Engine>,
    commands: Option<mpsc::Sender<Command>>,
    worker: Option<thread::JoinHandle<()>>,
    call_to_play: Arc<AtomicU64>,
    last_number: u64,
    call: Option<u64>,
    join_came_back: bool,
    join_asked: Instant,
    up_since: Option<Instant>,
    pending_leaves: usize,
    last_left_call: u64,
    watched: Option<String>,
    meter: Meter,
    listening: bool,
    channel: Channel,
    phase: Phase,
    present: Vec<Member>,
    muted: bool,
    notice: Option<Notice>,
}

#[cfg(feature = "harness")]
#[derive(Clone, Debug)]
pub struct Stage {
    pub phase: Phase,
    pub you: Option<MemberId>,
    pub members: Vec<Member>,
    pub speaking: Vec<MemberId>,
    pub present: Vec<Member>,
    pub muted: bool,
    pub notice: Option<Notice>,
    pub up_for: Option<std::time::Duration>,
}

impl Call {
    #[cfg(feature = "harness")]
    pub fn staged(stage: Stage) -> io::Result<(Self, UnboundedReceiver<Happening>)> {
        let (happenings, received) = unbounded();
        let engine = Arc::new(Engine::new(Conditioning::Engine, Arc::new(Forward(happenings)))?);
        let mut call = Self {
            engine,
            commands: None,
            worker: None,
            call_to_play: Arc::new(AtomicU64::new(0)),
            last_number: 0,
            call: None,
            join_came_back: false,
            join_asked: Instant::now(),
            up_since: None,
            pending_leaves: 0,
            last_left_call: 0,
            watched: None,
            meter: Meter::default(),
            listening: false,
            channel: Channel::default(),
            phase: Phase::Idle,
            present: Vec::new(),
            muted: false,
            notice: None,
        };
        call.stage(stage);
        Ok((call, received))
    }

    #[cfg(feature = "harness")]
    pub fn stage(&mut self, stage: Stage) {
        let state = match stage.phase {
            Phase::Connected => State::Connected,
            Phase::Reconnecting => State::Reconnecting,
            Phase::Idle | Phase::Joining | Phase::Failed(_) => State::Out,
        };
        self.channel = Channel::shown(state, stage.you, stage.members, stage.speaking);
        self.join_came_back = matches!(stage.phase, Phase::Connected | Phase::Reconnecting);
        self.phase = stage.phase;
        self.present = stage.present;
        self.muted = stage.muted;
        self.notice = stage.notice;
        self.up_since = stage.up_for.map(|up_for| Instant::now() - up_for);
    }

    pub fn new(player: impl Player) -> io::Result<(Self, UnboundedReceiver<Happening>)> {
        let (happenings, received) = unbounded();
        // Desktop devices pass the microphone's signal unprocessed.
        let engine = Arc::new(Engine::new(Conditioning::Engine, Arc::new(Forward(happenings.clone())))?);
        let (commands, inbox) = mpsc::channel();
        let call_to_play = Arc::new(AtomicU64::new(0));
        let meter = Meter::default();
        let worker = Worker {
            engine: Arc::clone(&engine),
            player: Box::new(player),
            happenings,
            call_to_play: Arc::clone(&call_to_play),
            meter: meter.clone(),
            devices: Devices::default(),
            listening: false,
            playing: None,
            listened: None,
        };
        let worker = thread::Builder::new().name("bukospeak-call".into()).spawn(move || worker.run(&inbox))?;
        let call = Self {
            engine,
            commands: Some(commands),
            worker: Some(worker),
            call_to_play,
            last_number: 0,
            call: None,
            join_came_back: false,
            join_asked: Instant::now(),
            up_since: None,
            pending_leaves: 0,
            last_left_call: 0,
            watched: None,
            meter,
            listening: false,
            channel: Channel::default(),
            phase: Phase::Idle,
            present: Vec::new(),
            muted: false,
            notice: None,
        };
        Ok((call, received))
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn is_in_call(&self) -> bool {
        matches!(self.phase, Phase::Connected | Phase::Reconnecting)
    }

    pub fn since(&self) -> Option<Instant> {
        self.up_since
    }

    pub fn you(&self) -> Option<MemberId> {
        self.channel.you()
    }

    pub fn members(&self) -> &[Member] {
        self.channel.members()
    }

    pub fn speaking(&self) -> &[MemberId] {
        self.channel.speaking()
    }

    pub fn present(&self) -> &[Member] {
        &self.present
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn join(&mut self, join: Join) {
        if self.call.is_some() {
            return;
        }
        self.last_number += 1;
        let number = self.last_number;
        self.call = Some(number);
        (self.join_came_back, self.channel) = (false, Channel::default());
        self.phase = Phase::Joining;
        self.notice = None;
        self.call_to_play.store(number, Ordering::Relaxed);
        self.join_asked = Instant::now();
        // The engine stops watching on a join.
        self.send(Command::Join { number, join, asked: self.join_asked });
    }

    pub fn leave(&mut self) {
        self.end(None);
    }

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

    pub fn set_suppression(&self, suppression: Suppression) {
        if let Err(error) = self.engine.set_suppression(suppression) {
            tracing::warn!("making the noise suppression chosen: {error}");
        }
    }

    pub fn set_echo_cancellation(&self, on: bool) {
        self.engine.set_echo_cancellation(on);
    }

    pub fn set_volume(&self, percent: u16) {
        self.engine.set_volume(f32::from(percent) / 100.0);
    }

    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    pub fn listen(&mut self, on: bool) {
        if self.listening != on {
            self.listening = on;
            self.send(Command::Listen(on));
        }
    }

    pub fn set_device(&self, direction: Direction, device: Option<DeviceId>) {
        self.send(Command::SetDevice(direction, device));
    }

    pub fn watch(&mut self, server: Option<String>) {
        if self.watched != server {
            self.watched = server;
            self.rewatch();
        }
    }

    pub fn happen(&mut self, happening: Happening) {
        match happening.0 {
            Event::Update(update) => self.update(*update),
            Event::Joined(number, result) => self.joined(number, result),
            Event::Left(call) => {
                self.pending_leaves -= 1;
                self.last_left_call = call;
            }
            Event::Device(number, error) => {
                tracing::warn!("{error}");
                if self.call == Some(number) {
                    self.notice = Some(notice(&error));
                }
            }
        }
    }

    fn joined(&mut self, number: u64, result: Result<(), Failure>) {
        if self.call != Some(number) {
            return;
        }
        match result {
            Ok(()) => {
                self.join_came_back = true;
                self.follow();
            }
            // The engine holds a call for the audio that didn't start, which only a leave ends.
            Err(Failure::Audio) => self.end(Some(Failure::Audio)),
            Err(failure) => {
                self.call = None;
                self.call_to_play.store(0, Ordering::Relaxed);
                self.phase = Phase::Failed(failure);
                self.rewatch();
            }
        }
    }

    fn update(&mut self, update: Update) {
        let Update { channel, lost, .. } = update;
        self.present = channel.present().to_vec();
        if self.is_stale(&channel) {
            return;
        }
        if let Some(lost) = lost {
            tracing::warn!("the call ended: {lost}");
            return self.end(Some(Failure::from(&lost)));
        }
        self.channel = channel;
        self.follow();
    }

    fn is_stale(&self, channel: &Channel) -> bool {
        self.call.is_none() || self.pending_leaves > 0 || channel.call() <= self.last_left_call
    }

    fn follow(&mut self) {
        if !self.join_came_back {
            return;
        }
        match self.channel.state() {
            State::Connected => {
                if self.phase == Phase::Joining {
                    log_step("the call shows connected", self.join_asked);
                    self.up_since = Some(Instant::now());
                }
                self.phase = Phase::Connected;
            }
            State::Reconnecting => self.phase = Phase::Reconnecting,
            // The update that tells who is in the call is still on its way.
            State::Out => {}
        }
    }

    fn end(&mut self, failure: Option<Failure>) {
        if self.call.take().is_none() {
            return;
        }
        self.call_to_play.store(0, Ordering::Relaxed);
        self.channel = Channel::default();
        self.up_since = None;
        self.phase = failure.map_or(Phase::Idle, Phase::Failed);
        self.pending_leaves += 1;
        self.send(Command::Leave);
        self.rewatch();
    }

    fn rewatch(&mut self) {
        if self.call.is_some() {
            return;
        }
        let command = match &self.watched {
            Some(server) => Command::Watch(server.clone()),
            None => Command::Unwatch,
        };
        self.send(command);
    }

    fn send(&self, command: Command) {
        // A staged call has no worker.
        if let Some(commands) = &self.commands {
            commands.send(command).expect("the worker runs while the call is there");
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.call_to_play.store(0, Ordering::Relaxed);
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            worker.join().expect("the worker doesn't panic");
        }
    }
}

fn notice(error: &audio::Error) -> Notice {
    use audio::Error;
    match *error {
        Error::Missing(direction, _) | Error::NoDevice(direction) => Notice::DeviceMissing(direction),
        Error::NoFormat(direction) | Error::Format(direction, _) | Error::Device(direction, _) => Notice::DeviceFailed(direction),
    }
}

fn log_step(step: &str, asked: Instant) {
    tracing::info!("{step}: {} ms after the join was asked for", asked.elapsed().as_millis());
}

struct Forward(UnboundedSender<Happening>);

impl Listener for Forward {
    fn update(&self, update: Update) {
        // Closed only once the call is dropped.
        let _ = self.0.unbounded_send(Happening(Event::Update(Box::new(update))));
    }
}

struct Worker {
    engine: Arc<Engine>,
    player: Box<dyn Player>,
    happenings: UnboundedSender<Happening>,
    call_to_play: Arc<AtomicU64>,
    meter: Meter,
    devices: Devices,
    listening: bool,
    playing: Option<Box<dyn Playing>>,
    listened: Option<Box<dyn Playing>>,
}

impl Worker {
    fn run(mut self, inbox: &mpsc::Receiver<Command>) {
        for command in inbox {
            match command {
                Command::Join { number, join, asked } => {
                    self.playing = self.join(number, join, asked);
                    self.relisten();
                }
                Command::Leave => {
                    // The audio stops before the call, so nothing plays into a call that is gone.
                    self.playing = None;
                    let left = self.engine.leave();
                    self.tell(Event::Left(left));
                    self.relisten();
                }
                Command::Watch(server) => self.engine.watch(server),
                Command::Unwatch => self.engine.unwatch(),
                Command::Listen(on) => {
                    self.listening = on;
                    self.relisten();
                }
                Command::SetDevice(direction, device) => {
                    for playing in [&self.playing, &self.listened].into_iter().flatten() {
                        playing.set_device(direction, device.clone());
                    }
                    match direction {
                        Direction::Input => self.devices.input = device,
                        Direction::Output => self.devices.output = device,
                    }
                }
            }
        }
        self.playing = None;
        self.engine.leave();
    }

    fn relisten(&mut self) {
        if !self.listening || self.playing.is_some() {
            self.listened = None;
        } else if self.listened.is_none() {
            // The next call tells the user why the input doesn't play.
            let report: Report = Box::new(|error| tracing::warn!("listening: {error}"));
            self.listened = self
                .player
                .listen(&self.meter, self.devices.input.as_ref(), report)
                .inspect_err(|error| tracing::warn!("the input isn't listened to: {error}"))
                .ok();
        }
    }

    fn join(&mut self, number: u64, join: Join, asked: Instant) -> Option<Box<dyn Playing>> {
        let Join { server, devices } = join;
        self.devices = devices.clone();
        let Joined { microphone, speaker } = match self.engine.join(&server) {
            Ok(joined) => {
                log_step("the server welcomed the join", asked);
                joined
            }
            Err(error) => {
                tracing::warn!("joining {server}: {error}");
                return self.joined(number, Err(Failure::from(&error)));
            }
        };
        if self.call_to_play.load(Ordering::Relaxed) != number {
            // Left while joining: the leave queued behind this join ends the call.
            return None;
        }
        let happenings = self.happenings.clone();
        let report: Report = Box::new(move |error| {
            // Closed only once the call is dropped.
            let _ = happenings.unbounded_send(Happening(Event::Device(number, error)));
        });
        self.listened = None;
        match self.player.play(microphone, speaker, &self.meter, &devices, report) {
            Ok(playing) => {
                log_step("the call's audio plays", asked);
                self.tell(Event::Joined(number, Ok(())));
                Some(playing)
            }
            Err(error) => {
                tracing::warn!("the audio didn't start: {error}");
                self.joined(number, Err(Failure::Audio))
            }
        }
    }

    fn joined(&self, number: u64, result: Result<(), Failure>) -> Option<Box<dyn Playing>> {
        self.tell(Event::Joined(number, result));
        None
    }

    fn tell(&self, event: Event) {
        // Closed only once the call is dropped.
        let _ = self.happenings.unbounded_send(Happening(event));
    }
}
