//! The BukoSpeak desktop app, for Windows, macOS and Linux.

// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;
mod ui;

use std::io::IsTerminal;

use bukospeak_desktop::call::{Call, DesktopPlayer};
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_desktop::settings::Settings;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext, Entity, KeyBinding, QuitMode};
use tracing_subscriber::EnvFilter;

use crate::tray::{Action, Tray};
use crate::ui::{Back, CONTEXT, Client, Trouble};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| bukospeak_desktop::LOG_FILTER.into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    // Every desktop system has one; without a home there is nowhere to keep anything.
    let folder = bukospeak_desktop::config_folder().expect("the system has a config folder");
    let settings_path = folder.join("settings.toml");
    let (settings, trouble) = match Settings::load(&settings_path) {
        Ok(settings) => (settings, None),
        Err(error) => {
            tracing::warn!("{error}");
            (Settings::default(), Some(Trouble::Unreadable))
        }
    };
    let (call, happenings) = Call::new(KnownServers::at(folder.join("known_servers")), DesktopPlayer).expect("the engine's threads start");

    gpui_kit::application().with_assets(ui::Assets).run(move |cx| {
        gpui_kit::init(cx);
        ui::theme::install(cx);
        cx.bind_keys([KeyBinding::new("escape", Back, Some(CONTEXT))]);
        let client = cx.new(|cx| Client::new(call, happenings, settings, settings_path, trouble, cx));
        let tray = show_tray(&client, cx);
        // Without a tray the app is its window: closing it leaves the call and quits. With one,
        // the app goes on in the tray until it is told to quit there.
        cx.set_quit_mode(if tray { QuitMode::Explicit } else { QuitMode::LastWindowClosed });
        ui::open(client, cx);
    });
}

/// Shows the app in the tray, if the system has one, and does what is asked there; true if it
/// is shown.
fn show_tray(client: &Entity<Client>, cx: &mut App) -> bool {
    let (actions, mut asked) = mpsc::unbounded();
    let Some(tray) = Tray::new(client.read(cx).call.is_muted(), actions) else {
        return false;
    };
    // The menu's check follows the mute however it changes; the tray lives as long as this.
    cx.observe(client, move |client, cx| tray.set_muted(client.read(cx).call.is_muted())).detach();
    let client = client.clone();
    cx.spawn(async move |cx| {
        while let Some(action) = asked.next().await {
            cx.update(|cx| match action {
                Action::Open => match cx.windows().first() {
                    Some(window) => {
                        // The window is there, so updating it can't fail for its absence.
                        let _ = window.update(cx, |_, window, _| window.activate_window());
                    }
                    None => ui::open(client.clone(), cx),
                },
                Action::ToggleMute => client.update(cx, |client, cx| client.toggle_mute(cx)),
                Action::Quit => cx.quit(),
            });
        }
    })
    .detach();
    true
}
