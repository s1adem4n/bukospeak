//! The BukoSpeak desktop app, for Windows, macOS and Linux.

// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

use std::io::IsTerminal;

use bukospeak_desktop::call::{Call, DesktopPlayer};
use bukospeak_desktop::known_servers::KnownServers;
use bukospeak_desktop::settings::Settings;
use gpui_kit::component::{Theme, TitleBar};
use gpui_kit::{AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, px, size};
use tracing_subscriber::EnvFilter;

use crate::ui::{Back, CONTEXT, Main, Trouble};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
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
        // The app is its window: closing it leaves the call and quits.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(380.), px(560.)), cx))),
            window_min_size: Some(size(px(320.), px(440.))),
            app_id: Some("bukospeak".into()),
            // The content draws the title bar, so it reaches up to the window's edge.
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.new(|cx| Main::new(call, happenings, settings, Some(settings_path), trouble, window, cx))
        })
        .expect("the window opens");
        cx.activate(true);
    });
}
