//! The BukoSpeak desktop app, for Windows, macOS and Linux.

// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkey;
mod instance;
mod tray;

use std::io::IsTerminal;
use std::sync::Arc;

use bukospeak_desktop::call::{Call, DesktopPlayer};
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_desktop::settings::Settings;
use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use gpui::{App, AppContext, Entity, QuitMode};
use tracing_subscriber::EnvFilter;

use crate::hotkey::{self as keys, Hotkey};
use crate::tray::{Standing, Tray};
use bukospeak_desktop::autopilot::Autopilot;
use bukospeak_desktop::autostart::BACKGROUND;
use bukospeak_desktop::call::Phase;
use bukospeak_desktop::ui::{self, Client, Desktop, RoomKeyRead, Trouble};
use bukospeak_desktop::update::{self, Updater};

/// What the tray, the hotkey or a later start asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    /// Joins the call, or leaves it if in it or joining.
    ToggleCall,
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
    // The system starts the app so, with it: into the tray, unless the window is needed.
    let background = std::env::args().any(|argument| argument == BACKGROUND);
    let sounds = folder.join("sounds");

    let app = gpui_platform::application().with_assets(ui::Assets);
    // The dock's icon, clicked while the window is closed.
    let reopen = actions.clone();
    app.on_reopen(move |_| send(&reopen, Action::Open));
    app.run(move |cx| {
        ui::init(cx);
        // Wayland has no keys for every app; GPUI on X11 or another system says something else.
        let hotkey = (cx.compositor_name() != "Wayland").then(|| Hotkey::new(actions.clone())).flatten();
        let mute_keys = hotkey.as_ref().map(|_| keys::KEYS);
        let client = cx.new(|cx| Client::new(call, happenings, Arc::new(Desktop::new(settings_path)), settings, trouble, mute_keys, cx));
        let updater = Updater::new(client.clone(), cx);
        client.update(cx, |client, cx| client.start_with_system(cx));
        let tray = show_tray(&client, actions.clone(), cx);
        // Without a tray the app is its window: closing it leaves the call and quits. With one,
        // the app goes on in the tray until it is told to quit there.
        cx.set_quit_mode(if tray { QuitMode::Explicit } else { QuitMode::LastWindowClosed });
        // It lives as long as the app.
        let autopilot = Autopilot::new(client.clone(), sounds, cx);
        follow(asked, client.clone(), updater.clone(), hotkey, autopilot, cx);
        // Without a tray, or with something to set up first, the window is needed; the room key
        // only tells once the keyring answered.
        if background && tray && client.read(cx).missing().is_none() {
            let (asking, updating) = (client.clone(), updater.clone());
            cx.subscribe(&client, move |_, _: &RoomKeyRead, cx| {
                if asking.read(cx).missing().is_some() {
                    open(&asking, &updating, cx);
                }
            })
            .detach();
        } else {
            ui::start(client, updater, cx);
        }
    });
}

/// Shows the app in the tray, if the system has one, following where one stands in the call;
/// true if it is shown.
fn show_tray(client: &Entity<Client>, actions: UnboundedSender<Action>, cx: &mut App) -> bool {
    let Some(tray) = Tray::new(actions) else {
        return false;
    };
    let show = move |client: &Client| {
        let call = &client.call;
        let standing = match (call.phase(), call.is_muted()) {
            (Phase::Idle | Phase::Failed(_), _) => Standing::Out,
            (_, true) => Standing::Muted,
            (_, false) => Standing::In,
        };
        tray.show(standing, call.is_muted());
    };
    show(client.read(cx));
    // The tray lives as long as this.
    cx.observe(client, move |client, cx| show(client.read(cx))).detach();
    true
}

/// Does what is asked, as long as the app runs; the hotkey stays registered as long.
fn follow(
    mut asked: UnboundedReceiver<Action>,
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    hotkey: Option<Hotkey>,
    autopilot: Entity<Autopilot>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let (_hotkey, _autopilot) = (hotkey, autopilot);
        while let Some(action) = asked.next().await {
            cx.update(|cx| match action {
                Action::Open => open(&client, &updater, cx),
                Action::ToggleCall => {
                    let read = client.read(cx);
                    if !matches!(read.call.phase(), Phase::Idle | Phase::Failed(_)) {
                        client.update(cx, |client, cx| client.leave(cx));
                    } else if read.missing().is_none() && !read.room_key().is_empty() {
                        client.update(cx, |client, cx| client.join(cx));
                    } else {
                        // What joining needs is given in the window.
                        open(&client, &updater, cx);
                    }
                }
                Action::ToggleMute => client.update(cx, |client, cx| client.toggle_mute(cx)),
                Action::Quit => cx.quit(),
            });
        }
    })
    .detach();
}

/// Shows the window, opening it if it isn't.
fn open(client: &Entity<Client>, updater: &Option<Entity<Updater>>, cx: &mut App) {
    match cx.windows().first() {
        Some(window) => {
            // The window is there, so updating it can't fail for its absence.
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        None => ui::open(client.clone(), updater.clone(), cx),
    }
}
