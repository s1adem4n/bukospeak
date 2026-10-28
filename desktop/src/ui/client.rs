//! What outlives the window: the call, the settings and the room key. The window shows them
//! and the tray and the hotkey act on them, with or without a window open.

use std::path::PathBuf;

use bukospeak_desktop::audio::Direction;
use bukospeak_desktop::call::{Call, Happening, Phase};
use bukospeak_desktop::settings::{self as stored, Settings};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::{AppContext, Context, EventEmitter, Task};

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

pub struct Client {
    pub call: Call,
    pub settings: Settings,
    settings_path: PathBuf,
    /// None until the keyring answered, or a key is given before.
    room_key: Option<String>,
    pub trouble: Option<Trouble>,
    /// The keys that mute and unmute from any app, if the system took them.
    pub mute_keys: Option<&'static str>,
    /// Whether a window shows the channel from outside, so who is in it is watched.
    lobby: bool,
    _happenings: Task<()>,
    _room_key: Task<()>,
}

impl Client {
    pub fn new(
        call: Call,
        mut happenings: UnboundedReceiver<Happening>,
        settings: Settings,
        settings_path: PathBuf,
        trouble: Option<Trouble>,
        mute_keys: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Self {
        call.set_noise_suppression(settings.noise_suppression);
        call.set_echo_cancellation(settings.echo_cancellation);
        call.set_volume(settings.volume);
        let happenings = cx.spawn(async move |this, cx| {
            while let Some(happening) = happenings.next().await {
                let applied = this.update(cx, |client, cx| {
                    client.call.happen(happening);
                    client.watch();
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        // The keyring may ask the system, and the system the user: not on the app's thread.
        let room_key = cx.spawn(async move |this, cx| {
            let read = cx.background_spawn(async { stored::room_key() }).await;
            // The app may be quitting before the keyring answered.
            let _ = this.update(cx, |client, cx| client.read_room_key(read, cx));
        });
        Self {
            call,
            settings,
            settings_path,
            room_key: None,
            trouble,
            mute_keys,
            lobby: false,
            _happenings: happenings,
            _room_key: room_key,
        }
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
        cx.spawn(async move |this, cx| {
            let kept = cx.background_spawn(async move { stored::set_room_key(&room_key) }).await;
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
        if let Err(error) = self.settings.save(&self.settings_path) {
            tracing::warn!("{error}");
            self.trouble = Some(Trouble::NotSaved);
        }
        cx.notify();
    }

    pub fn set_device(&mut self, direction: Direction, id: Option<String>, cx: &mut Context<Self>) {
        self.change(|settings| settings.set_device_id(direction, id), cx);
        self.call.set_device(direction, self.settings.device(direction));
    }

    pub fn set_noise_suppression(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|settings| settings.noise_suppression = on, cx);
        self.call.set_noise_suppression(on);
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
