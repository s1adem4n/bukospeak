//! The BukoSpeak desktop app, for Windows, macOS and Linux.

// A release started from the desktop opens no console window beside its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::IsTerminal;
use std::sync::Arc;

use gpui_kit::assets::Assets;
use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::{
    AppContext, Bounds, Context, FontWeight, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Render, Styled, StyledImage,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, px, size,
};
use tracing_subscriber::EnvFilter;

/// The logo, as the iOS app shows it: one picture for both apps.
const BRAND_MARK: &[u8] = include_bytes!("../../ios/BukoSpeak/Assets.xcassets/BrandMark.imageset/BrandMark.png");

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    gpui_kit::application().with_assets(Assets).run(|cx| {
        gpui_kit::init(cx);
        // The app is its window: closing it quits.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(420.), px(640.)), cx))),
            window_min_size: Some(size(px(360.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some("BukoSpeak".into()), ..Default::default() }),
            app_id: Some("bukospeak".into()),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.new(|cx| Lobby::new(window, cx))
        })
        .expect("the window opens");
        cx.activate(true);
    });
}

/// Before a call: the logo and the app's name.
struct Lobby {
    brand_mark: Arc<Image>,
    _appearance: Subscription,
}

impl Lobby {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        Self { brand_mark: Arc::new(Image::from_bytes(ImageFormat::Png, BRAND_MARK.to_vec())), _appearance: appearance }
    }
}

impl Render for Lobby {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(img(self.brand_mark.clone()).h(px(200.)).object_fit(ObjectFit::Contain))
            .child(div().text_3xl().font_weight(FontWeight::BOLD).child("BukoSpeak"))
    }
}
