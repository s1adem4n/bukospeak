use std::sync::Arc;

use crate::audio::Direction;
use crate::call::{Call, Happening, Phase};
use crate::nvidia::{Architecture, Effects};
use crate::settings::{Settings, Suppression};
use crate::ui::host::Host;
use bukospeak_core::Error;
use bukospeak_core::install::{self, Progress};
use bukospeak_core::proto::control::Rejection;
use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedReceiver};
use gpui::{AppContext, Context, Task};

use crate::ui::Trouble;

pub struct Nvidia {
    architecture: Architecture,
    effects: Option<Arc<Effects>>,
    pub installing: Option<Progress>,
    failed: mpsc::UnboundedSender<()>,
}

impl Nvidia {
    fn installed(&self) -> Option<Arc<Effects>> {
        let failed = self.failed.clone();
        Effects::installed(self.architecture, move || _ = failed.unbounded_send(()))
    }
}

enum Installing {
    Progress(Progress),
    Done(Result<(), install::Error>),
}

pub struct Client {
    pub call: Call,
    pub settings: Settings,
    host: Arc<dyn Host>,
    pub trouble: Option<Trouble>,
    pub mute_keys: Option<&'static str>,
    lobby: bool,
    pub nvidia: Option<Nvidia>,
    syncing: Option<Task<()>>,
    _happenings: Task<()>,
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
        // CUDA takes a moment to tell the card.
        let system = Arc::clone(&host);
        let card = cx.spawn(async move |this, cx| {
            if let Some(architecture) = cx.background_spawn(async move { system.card() }).await {
                let _ = this.update(cx, |client, cx| client.found(architecture, cx));
            }
        });
        Self { call, settings, host, trouble, mute_keys, lobby: false, nvidia: None, syncing: None, _happenings: happenings, _card: card }
    }

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

    fn install(&mut self, architecture: Architecture, cx: &mut Context<Self>) {
        let server = self.settings.server().to_owned();
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
                let installed = host.install(&server, architecture, &mut |now| {
                    // Each percent, not each chunk: the window redraws for each.
                    let percent = match now {
                        Progress::Preparing { done, total } | Progress::Fetching { done, total } => (done * 100).checked_div(total),
                    };
                    if shown.replace((std::mem::discriminant(&now), percent)) != Some((std::mem::discriminant(&now), percent)) {
                        // The app may be quitting; the installation runs to the end regardless.
                        let _ = progress.unbounded_send(Installing::Progress(now));
                    }
                });
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

    fn sync_sounds(&mut self, cx: &mut Context<Self>) {
        if self.syncing.is_some() {
            return;
        }
        let server = self.settings.server().to_owned();
        let host = Arc::clone(&self.host);
        self.syncing = Some(cx.spawn(async move |this, cx| {
            let synced = cx.background_spawn(async move { host.sync_sounds(&server) }).await;
            // Without them, the cues are the app's own tones.
            if let Err(error) = synced {
                tracing::warn!("installing the server's sounds: {error}");
            }
            let _ = this.update(cx, |client, _| client.syncing = None);
        }));
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    pub fn join(&mut self, cx: &mut Context<Self>) {
        self.trouble = None;
        self.call.join(self.settings.join());
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

    pub fn set_lobby(&mut self, lobby: bool) {
        self.lobby = lobby;
        self.watch();
    }

    pub fn set_listening(&mut self, listening: bool) {
        self.call.listen(listening);
    }

    fn watch(&mut self) {
        let outside = matches!(self.call.phase(), Phase::Idle | Phase::Failed(_)) && self.lobby;
        let server = (outside && self.settings.is_complete()).then(|| self.settings.server().to_owned());
        self.call.watch(server);
    }

    pub fn set_server(&mut self, server: String, cx: &mut Context<Self>) {
        self.change(|settings| settings.server = server, cx);
        self.watch();
    }

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

    pub fn start_with_system(&mut self, cx: &mut Context<Self>) {
        let (host, on) = (Arc::clone(&self.host), self.settings.start_with_system);
        cx.spawn(async move |this, cx| {
            if let Err(error) = cx.background_spawn(async move { host.start_with_system(on) }).await {
                tracing::warn!("starting with the system: {error}");
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
