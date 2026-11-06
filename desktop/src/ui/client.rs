//! What outlives the window: the call, the settings and the room key. The window shows them
//! and the tray and the hotkey act on them, with or without a window open.

use std::sync::Arc;

use crate::audio::Direction;
use crate::call::{Call, Happening, Phase};
use crate::install::{self, Progress};
use crate::nvidia::{Architecture, Effects};
use crate::settings::{self as stored, Settings, Suppression};
use crate::ui::host::Host;
use bukospeak_core::Error;
use bukospeak_core::proto::control::Rejection;
use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedReceiver};
use gpui::{AppContext, Context, EventEmitter, Task};

use crate::ui::Trouble;

/// The room key came from the keyring, or didn't: [`Client::room_key`] is known from now on.
pub struct RoomKeyRead;

/// What is missing to join, in the order the welcome asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The name the others see.
    Name,
    /// The server and its room key.
    Place,
}

impl EventEmitter<RoomKeyRead> for Client {}

/// NVIDIA's effects on the system's RTX card.
pub struct Nvidia {
    architecture: Architecture,
    /// The effects, once installed.
    effects: Option<Arc<Effects>>,
    /// How far the installation going on got.
    pub installing: Option<Progress>,
    /// Tells the app that an effect didn't start.
    failed: mpsc::UnboundedSender<()>,
}

impl Nvidia {
    /// The effects, if installed.
    fn installed(&self) -> Option<Arc<Effects>> {
        let failed = self.failed.clone();
        // The app hears of it as long as it runs.
        Effects::installed(self.architecture, move || _ = failed.unbounded_send(()))
    }
}

/// What the installation's thread tells.
enum Installing {
    Progress(Progress),
    Done(Result<(), install::Error>),
}

pub struct Client {
    pub call: Call,
    pub settings: Settings,
    host: Arc<dyn Host>,
    /// None until the keyring answered, or a key is given before.
    room_key: Option<String>,
    pub trouble: Option<Trouble>,
    /// The keys that mute and unmute from any app, if the system took them.
    pub mute_keys: Option<&'static str>,
    /// Whether a window shows the channel from outside, so who is in it is watched.
    lobby: bool,
    /// NVIDIA's effects, once the system's RTX card was found.
    pub nvidia: Option<Nvidia>,
    /// The server's sounds being installed, if they are.
    syncing: Option<Task<()>>,
    _happenings: Task<()>,
    _room_key: Task<()>,
    _card: Task<()>,
}

impl Client {
    pub fn new(
        call: Call,
        mut happenings: UnboundedReceiver<Happening>,
        host: Arc<dyn Host>,
        settings: Settings,
        trouble: Option<Trouble>,
        mute_keys: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Self {
        call.set_device(Direction::Input, settings.device(Direction::Input));
        call.set_device(Direction::Output, settings.device(Direction::Output));
        call.set_suppression(settings.suppression.engines(None));
        call.set_echo_cancellation(settings.echo_cancellation);
        call.set_volume(settings.volume);
        let happenings = cx.spawn(async move |this, cx| {
            while let Some(happening) = happenings.next().await {
                let applied = this.update(cx, |client, cx| {
                    let connected = *client.call.phase() == Phase::Connected;
                    client.call.happen(happening);
                    if !connected && *client.call.phase() == Phase::Connected {
                        client.sync_sounds(cx);
                    }
                    client.watch();
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        // The keyring may ask the system, and the system the user: not on the app's thread.
        let keyring = Arc::clone(&host);
        let room_key = cx.spawn(async move |this, cx| {
            let read = cx.background_spawn(async move { keyring.room_key() }).await;
            // The app may be quitting before the keyring answered.
            let _ = this.update(cx, |client, cx| client.read_room_key(read, cx));
        });
        // CUDA takes a moment to tell the card.
        let system = Arc::clone(&host);
        let card = cx.spawn(async move |this, cx| {
            if let Some(architecture) = cx.background_spawn(async move { system.card() }).await {
                // The app may be quitting before the card was found.
                let _ = this.update(cx, |client, cx| client.found(architecture, cx));
            }
        });
        Self {
            call,
            settings,
            host,
            room_key: None,
            trouble,
            mute_keys,
            lobby: false,
            nvidia: None,
            syncing: None,
            _happenings: happenings,
            _room_key: room_key,
            _card: card,
        }
    }

    /// Takes in the system's RTX card: NVIDIA's effects suppress the noise from now on if they
    /// are chosen, once installed.
    fn found(&mut self, architecture: Architecture, cx: &mut Context<Self>) {
        let (failed, failures) = mpsc::unbounded();
        cx.spawn(async move |this, cx| {
            let mut failures = failures;
            while failures.next().await.is_some() {
                let told = this.update(cx, |client, cx| {
                    client.trouble = Some(Trouble::NvidiaFailed);
                    cx.notify();
                });
                if told.is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut nvidia = Nvidia { architecture, effects: None, installing: None, failed };
        nvidia.effects = nvidia.installed();
        self.nvidia = Some(nvidia);
        self.suppress(cx);
    }

    /// Suppresses the noise as chosen, installing NVIDIA's effects first if they are chosen and
    /// not installed: Standard suppresses it meanwhile.
    fn suppress(&mut self, cx: &mut Context<Self>) {
        let effects = self.nvidia.as_ref().and_then(|nvidia| nvidia.effects.as_ref());
        self.call.set_suppression(self.settings.suppression.engines(effects));
        if self.settings.suppression.nvidia().is_some()
            && let Some(nvidia) = &self.nvidia
            && nvidia.effects.is_none()
            && nvidia.installing.is_none()
        {
            self.install(nvidia.architecture, cx);
        }
        cx.notify();
    }

    /// Installs NVIDIA's effects for `architecture` from the server, on a thread of its own:
    /// it may take minutes.
    fn install(&mut self, architecture: Architecture, cx: &mut Context<Self>) {
        let (server, room_key, known) =
            (self.settings.server.trim().to_owned(), self.room_key().to_owned(), self.call.known_servers().clone());
        if let Some(nvidia) = &mut self.nvidia {
            nvidia.installing = Some(Progress::Preparing { done: 0, total: 0 });
        }
        let host = Arc::clone(&self.host);
        let (tell, mut told) = mpsc::unbounded();
        let progress = tell.clone();
        std::thread::Builder::new()
            .name("bukospeak-nvidia".into())
            .spawn(move || {
                let mut shown = None;
                let installed = host.install(&server, room_key, &known, architecture, &mut |now| {
                    // Each percent, not each chunk: the window redraws for each.
                    let percent = match now {
                        Progress::Preparing { done, total } | Progress::Fetching { done, total } => (done * 100).checked_div(total),
                    };
                    if shown.replace((std::mem::discriminant(&now), percent)) != Some((std::mem::discriminant(&now), percent)) {
                        // The app may be quitting; the installation goes on to the end regardless.
                        let _ = progress.unbounded_send(Installing::Progress(now));
                    }
                });
                // As above.
                let _ = tell.unbounded_send(Installing::Done(installed));
            })
            .expect("the installation's thread starts");
        cx.spawn(async move |this, cx| {
            while let Some(told) = told.next().await {
                let applied = this.update(cx, |client, cx| match told {
                    Installing::Progress(progress) => {
                        if let Some(nvidia) = &mut client.nvidia {
                            nvidia.installing = Some(progress);
                        }
                        cx.notify();
                    }
                    Installing::Done(installed) => client.installed(installed, cx),
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Takes in how installing NVIDIA's effects ended: they suppress the noise if chosen, or
    /// Standard does once they couldn't be installed.
    fn installed(&mut self, installed: Result<(), install::Error>, cx: &mut Context<Self>) {
        let Some(nvidia) = &mut self.nvidia else { return };
        nvidia.installing = None;
        match installed {
            Ok(()) => nvidia.effects = nvidia.installed(),
            Err(error) => {
                tracing::warn!("installing NVIDIA's effects: {error}");
                self.trouble = Some(match error {
                    install::Error::Download(Error::Rejected(Rejection::Unavailable)) => Trouble::NvidiaUnavailable,
                    _ => Trouble::NvidiaNotInstalled,
                });
                if self.settings.suppression.nvidia().is_some() {
                    self.change(|settings| settings.suppression = Suppression::Standard, cx);
                }
            }
        }
        self.suppress(cx);
    }

    /// Installs the sounds the server of the call offers as it has them now, in the background,
    /// unless that is going on: the server is trusted then, and its room key right.
    fn sync_sounds(&mut self, cx: &mut Context<Self>) {
        if self.syncing.is_some() {
            return;
        }
        let (server, room_key, known) =
            (self.settings.server.trim().to_owned(), self.room_key().to_owned(), self.call.known_servers().clone());
        let host = Arc::clone(&self.host);
        self.syncing = Some(cx.spawn(async move |this, cx| {
            let synced = cx.background_spawn(async move { host.sync_sounds(&server, room_key, &known) }).await;
            // Without them, the cues are the app's own tones.
            if let Err(error) = synced {
                tracing::warn!("installing the server's sounds: {error}");
            }
            // The app may be quitting meanwhile.
            let _ = this.update(cx, |client, _| client.syncing = None);
        }));
    }

    /// Takes in the room key the keyring kept, unless one was given meanwhile.
    fn read_room_key(&mut self, read: Result<Option<String>, stored::Error>, cx: &mut Context<Self>) {
        let key = read.unwrap_or_else(|error| {
            tracing::warn!("the room key isn't readable: {error}");
            self.trouble = Some(Trouble::RoomKeyUnreadable);
            None
        });
        if self.room_key.is_none() {
            self.room_key = Some(key.unwrap_or_default());
            self.watch();
            cx.emit(RoomKeyRead);
            cx.notify();
        }
    }

    /// What the window asks of the system.
    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    /// The room key; empty until the keyring answered.
    pub fn room_key(&self) -> &str {
        self.room_key.as_deref().unwrap_or_default()
    }

    /// What is missing to join, if anything is; the room key only once the keyring answered.
    pub fn missing(&self) -> Option<Missing> {
        if self.settings.name.trim().is_empty() {
            Some(Missing::Name)
        } else if self.settings.server.trim().is_empty() || self.room_key.is_some() && !self.settings.is_complete(self.room_key()) {
            Some(Missing::Place)
        } else {
            None
        }
    }

    pub fn join(&mut self, cx: &mut Context<Self>) {
        self.trouble = None;
        self.call.join(self.settings.join(self.room_key()));
        cx.notify();
    }

    pub fn trust(&mut self, cx: &mut Context<Self>) {
        self.call.trust(self.settings.join(self.room_key()));
        cx.notify();
    }

    pub fn leave(&mut self, cx: &mut Context<Self>) {
        self.call.leave();
        self.watch();
        cx.notify();
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.call.set_muted(!self.call.is_muted());
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.call.dismiss();
        self.watch();
        cx.notify();
    }

    /// Whether a window shows the channel from outside.
    pub fn set_lobby(&mut self, lobby: bool) {
        self.lobby = lobby;
        self.watch();
    }

    /// Listens to the microphone while a window shows its level, out of a call too.
    pub fn set_listening(&mut self, listening: bool) {
        self.call.listen(listening);
    }

    /// Watches who is in the channel while a window shows it from outside.
    fn watch(&mut self) {
        let outside = matches!(self.call.phase(), Phase::Idle | Phase::Failed(_)) && self.lobby;
        let channel = (outside && self.settings.is_complete(self.room_key()))
            .then(|| (self.settings.server.trim().to_owned(), self.room_key().to_owned()));
        self.call.watch(channel);
    }

    pub fn set_name(&mut self, name: String, cx: &mut Context<Self>) {
        self.change(|settings| settings.name = name, cx);
    }

    /// Takes another server, which the channel is watched on from now on.
    pub fn set_server(&mut self, server: String, cx: &mut Context<Self>) {
        self.change(|settings| settings.server = server, cx);
        self.watch();
    }

    /// Takes another room key, kept in the keyring; the channel is watched with it from now on.
    pub fn set_room_key(&mut self, room_key: String, cx: &mut Context<Self>) {
        self.room_key = Some(room_key.clone());
        self.watch();
        cx.notify();
        let host = Arc::clone(&self.host);
        cx.spawn(async move |this, cx| {
            let kept = cx.background_spawn(async move { host.set_room_key(&room_key) }).await;
            if let Err(error) = kept {
                tracing::warn!("the room key isn't kept: {error}");
                // The app may be quitting before the keyring answered.
                let _ = this.update(cx, |client, cx| {
                    client.trouble = Some(Trouble::RoomKeyNotKept);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Changes the settings and keeps them.
    fn change(&mut self, edit: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        edit(&mut self.settings);
        if let Err(error) = self.host.save_settings(&self.settings) {
            tracing::warn!("{error}");
            self.trouble = Some(Trouble::NotSaved);
        }
        cx.notify();
    }

    pub fn set_device(&mut self, direction: Direction, id: Option<String>, cx: &mut Context<Self>) {
        self.change(|settings| settings.set_device_id(direction, id), cx);
        self.call.set_device(direction, self.settings.device(direction));
    }

    pub fn set_suppression(&mut self, suppression: Suppression, cx: &mut Context<Self>) {
        self.change(|settings| settings.suppression = suppression, cx);
        self.suppress(cx);
    }

    /// Has the system start the app with it or not, as the settings say: at the app's start,
    /// so an install moved keeps being started, and when they change.
    pub fn start_with_system(&mut self, cx: &mut Context<Self>) {
        let (host, on) = (Arc::clone(&self.host), self.settings.start_with_system);
        cx.spawn(async move |this, cx| {
            if let Err(error) = cx.background_spawn(async move { host.start_with_system(on) }).await {
                tracing::warn!("starting with the system: {error}");
                // The app may be quitting before the system answered.
                let _ = this.update(cx, |client, cx| {
                    client.trouble = Some(Trouble::StartNotSet);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn set_start_with_system(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|settings| settings.start_with_system = on, cx);
        self.start_with_system(cx);
    }

    /// Joins for `game` while it runs from now on, or no more.
    pub fn set_game(&mut self, game: &str, on: bool, cx: &mut Context<Self>) {
        self.change(
            |settings| {
                settings.games.retain(|kept| !kept.eq_ignore_ascii_case(game));
                if on {
                    settings.games.push(game.to_owned());
                }
            },
            cx,
        );
    }

    pub fn set_echo_cancellation(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|settings| settings.echo_cancellation = on, cx);
        self.call.set_echo_cancellation(on);
    }

    /// Plays the others at `percent` of their level, and keeps it once `done` choosing: while a
    /// slider is dragged, it is heard at once but saved only at the end.
    pub fn set_volume(&mut self, percent: u16, done: bool, cx: &mut Context<Self>) {
        self.call.set_volume(percent);
        if done {
            self.change(|settings| settings.volume = percent, cx);
        } else {
            self.settings.volume = percent;
            cx.notify();
        }
    }
}
