//! The BukoSpeak desktop app, for Windows, macOS and Linux.

// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkey;
mod instance;
mod tray;
mod ui;
mod update;

use std::io::IsTerminal;

use bukospeak_desktop::call::{Call, DesktopPlayer};
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_desktop::settings::Settings;
use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use gpui_kit::{App, AppContext, Entity, QuitMode};
use tracing_subscriber::EnvFilter;

use crate::hotkey::{self as keys, Hotkey};
use crate::tray::Tray;
use crate::ui::{Client, Trouble};
use crate::update::Updater;

/// What the tray, the hotkey or a later start asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleMute,
    Quit,
}

/// Sends an action to the app.
fn send(actions: &UnboundedSender<Action>, action: Action) {
    // A closed channel means the app is quitting, when nothing is left to do.
    let _ = actions.unbounded_send(action);
}

fn main() {
    update::start();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| bukospeak_desktop::LOG_FILTER.into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    // Every desktop system has one; without a home there is nowhere to keep anything.
    let folder = bukospeak_desktop::config_folder().expect("the system has a config folder");
    let (actions, asked) = mpsc::unbounded();
    if !instance::take(&folder, actions.clone()) {
        return;
    }
    let settings_path = folder.join("settings.toml");
    let (settings, trouble) = match Settings::load(&settings_path) {
        Ok(settings) => (settings, None),
        Err(error) => {
            tracing::warn!("{error}");
            (Settings::default(), Some(Trouble::Unreadable))
        }
    };
    let (call, happenings) = Call::new(KnownServers::at(folder.join("known_servers")), DesktopPlayer).expect("the engine's threads start");

    let app = gpui_kit::application().with_assets(ui::Assets);
    // The dock's icon, clicked while the window is closed.
    let reopen = actions.clone();
    app.on_reopen(move |_| send(&reopen, Action::Open));
    app.run(move |cx| {
        gpui_kit::init(cx);
        ui::init(cx);
        // Wayland has no keys for every app; GPUI on X11 or another system says something else.
        let hotkey = (cx.compositor_name() != "Wayland").then(|| Hotkey::new(actions.clone())).flatten();
        let mute_keys = hotkey.as_ref().map(|_| keys::KEYS);
        let client = cx.new(|cx| Client::new(call, happenings, settings, settings_path, trouble, mute_keys, cx));
        let updater = Updater::new(client.clone(), cx);
        let tray = show_tray(&client, actions.clone(), cx);
        // Without a tray the app is its window: closing it leaves the call and quits. With one,
        // the app goes on in the tray until it is told to quit there.
        cx.set_quit_mode(if tray { QuitMode::Explicit } else { QuitMode::LastWindowClosed });
        follow(asked, client.clone(), updater.clone(), hotkey, cx);
        ui::start(client, updater, cx);
    });
}

/// Shows the app in the tray, if the system has one, its check following the mute; true if it
/// is shown.
fn show_tray(client: &Entity<Client>, actions: UnboundedSender<Action>, cx: &mut App) -> bool {
    let Some(tray) = Tray::new(client.read(cx).call.is_muted(), actions) else {
        return false;
    };
    // The tray lives as long as this.
    cx.observe(client, move |client, cx| tray.set_muted(client.read(cx).call.is_muted())).detach();
    true
}

/// Does what is asked, as long as the app runs; the hotkey stays registered as long.
fn follow(
    mut asked: UnboundedReceiver<Action>,
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    hotkey: Option<Hotkey>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let _hotkey = hotkey;
        while let Some(action) = asked.next().await {
            cx.update(|cx| match action {
                Action::Open => match cx.windows().first() {
                    Some(window) => {
                        // The window is there, so updating it can't fail for its absence.
                        let _ = window.update(cx, |_, window, _| window.activate_window());
                    }
                    None => ui::open(client.clone(), updater.clone(), cx),
                },
                Action::ToggleMute => client.update(cx, |client, cx| client.toggle_mute(cx)),
                Action::Quit => cx.quit(),
            });
        }
    })
    .detach();
}
