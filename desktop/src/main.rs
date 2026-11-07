// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkey;
mod instance;
mod tray;

use std::io::IsTerminal;
use std::sync::Arc;

use bukospeak_desktop::call::{Call, DesktopPlayer};
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
use bukospeak_desktop::ui::{self, Client, Desktop, Trouble};
use bukospeak_desktop::update::{self, Updater};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleCall,
    ToggleMute,
    Quit,
}

fn send(actions: &UnboundedSender<Action>, action: Action) {
    // Closed only while the app quits.
    let _ = actions.unbounded_send(action);
}

fn main() {
    update::start();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| bukospeak_desktop::LOG_FILTER.into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
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
    let (call, happenings) = Call::new(DesktopPlayer).expect("the engine's threads start");
    let background = std::env::args().any(|argument| argument == BACKGROUND);

    let app = gpui_platform::application().with_assets(ui::Assets);
    // On macOS, the dock's icon clicked while the window is closed.
    let reopen = actions.clone();
    app.on_reopen(move |_| send(&reopen, Action::Open));
    app.run(move |cx| {
        ui::init(cx);
        // Wayland has no keys for every app.
        let hotkey = (cx.compositor_name() != "Wayland").then(|| Hotkey::new(actions.clone())).flatten();
        let mute_keys = hotkey.as_ref().map(|_| keys::KEYS);
        let client = cx.new(|cx| Client::new(call, happenings, Arc::new(Desktop::new(settings_path)), settings, trouble, mute_keys, cx));
        let updater = Updater::new(client.clone(), cx);
        client.update(cx, |client, cx| client.start_with_system(cx));
        let tray = show_tray(&client, actions.clone(), cx);
        cx.set_quit_mode(if tray { QuitMode::Explicit } else { QuitMode::LastWindowClosed });
        let autopilot = Autopilot::new(client.clone(), cx);
        follow(asked, client.clone(), updater.clone(), hotkey, autopilot, cx);
        let stays_in_tray = background && tray && client.read(cx).settings.is_complete();
        if !stays_in_tray {
            ui::start(client, updater, cx);
        }
    });
}

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
    cx.observe(client, move |client, cx| show(client.read(cx))).detach();
    true
}

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
                    } else if read.settings.is_complete() {
                        client.update(cx, |client, cx| client.join(cx));
                    } else {
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

fn open(client: &Entity<Client>, updater: &Option<Entity<Updater>>, cx: &mut App) {
    match cx.windows().first() {
        Some(window) => {
            // Fails only for a window that is gone.
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        None => ui::open(client.clone(), updater.clone(), cx),
    }
}
