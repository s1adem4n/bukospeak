//! The window: a welcome the first time, then the channel with who is in it and the dock to
//! join, mute and leave; the settings; and the dialog for a changed certificate. Before it, at
//! the start, the update window, while a newer release is looked for.

mod client;
mod glyph;
mod kit;
mod screens;
mod text;
mod theme;

use bukospeak_desktop::call::{Failure, Phase};
use gpui_kit::component::{Theme, TitleBar, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, Styled, Subscription, Window, WindowBounds, WindowOptions, actions, div, px, size,
};

pub use crate::ui::client::Client;
use crate::ui::client::{Missing, RoomKeyRead};
pub use crate::ui::glyph::Assets;
use crate::ui::glyph::Glyph;
use crate::ui::kit::{Button, Size, Variant};
use crate::ui::screens::{Channel, ChannelEvent, Settings, Update, Welcome, WelcomeEvent, certificate};
use crate::ui::theme::ActiveTokens;
use crate::update::Updater;

actions!(
    bukospeak,
    [
        /// Escape: closes the dialog or the settings, or goes a step back.
        Back
    ]
);

/// The key context of the window's content, which Back is bound in.
const CONTEXT: &str = "Main";

/// Installs the looks and binds the keys, once, before a window opens.
pub fn init(cx: &mut App) {
    theme::install(cx);
    kit::init(cx);
    cx.bind_keys([KeyBinding::new("escape", Back, Some(CONTEXT))]);
}

/// Opens the window at the start: with an updater, the update window first, which opens it.
pub fn start(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let Some(updater) = updater else {
        return open(client, None, cx);
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(300.), px(320.)), cx))),
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("bukospeak".into()),
        // No title bar: the window is moved by any of it.
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        Theme::sync_system_appearance(Some(window), cx);
        cx.new(|cx| Update::new(client, updater, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

/// Opens the window, which shows the client and a release the updater waits to install.
pub fn open(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(380.), px(560.)), cx))),
        window_min_size: Some(size(px(320.), px(440.))),
        app_id: Some("bukospeak".into()),
        // The content draws the title bar, so it reaches up to the window's edge.
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        Theme::sync_system_appearance(Some(window), cx);
        cx.new(|cx| Main::new(client, updater, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

/// What went wrong with the settings themselves, beside the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The settings file couldn't be read: the app starts from the defaults.
    Unreadable,
    NotSaved,
    /// The keyring couldn't be read: the room key is to be given anew.
    RoomKeyUnreadable,
    RoomKeyNotKept,
}

/// The screen the window shows.
enum Screen {
    Channel(Entity<Channel>),
    Welcome(Entity<Welcome>),
    Settings(Entity<Settings>),
}

pub struct Main {
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    screen: Screen,
    /// Where the focus rests between fields, so the window's keys reach it.
    focus: FocusHandle,
    /// What the window hears from its screen, if the screen tells it anything.
    _screen: Option<Subscription>,
    _subscriptions: [Subscription; 3],
}

impl Main {
    fn new(client: Entity<Client>, updater: Option<Entity<Updater>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |_, _, cx| cx.notify());
        let read = cx.subscribe_in(&client, window, |main, _, _: &RoomKeyRead, window, cx| main.read_room_key(window, cx));
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        // A closed window no longer shows who is in the channel.
        cx.on_release(|main, cx| main.client.update(cx, |client, _| client.set_lobby(false))).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let (screen, subscription) = match client.read(cx).missing() {
            Some(missing) => Self::welcome(&client, missing, window, cx),
            None => Self::channel(&client, updater.as_ref(), window, cx),
        };
        let main = Self { client, updater, screen, focus, _screen: Some(subscription), _subscriptions: [observed, read, appearance] };
        main.watch(cx);
        main
    }

    fn channel(
        client: &Entity<Client>,
        updater: Option<&Entity<Updater>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let channel = cx.new(|cx| Channel::new(client.clone(), updater.cloned(), cx));
        let subscription = cx.subscribe_in(&channel, window, |main, _, event: &ChannelEvent, window, cx| match event {
            ChannelEvent::Settings => main.show_settings(window, cx),
            ChannelEvent::Setup(missing) => main.show_welcome(*missing, window, cx),
        });
        (Screen::Channel(channel), subscription)
    }

    fn welcome(client: &Entity<Client>, missing: Missing, window: &mut Window, cx: &mut Context<Self>) -> (Screen, Subscription) {
        let welcome = cx.new(|cx| Welcome::new(client.clone(), missing, window, cx));
        let subscription = cx.subscribe_in(&welcome, window, |main, _, _: &WelcomeEvent, window, cx| {
            main.show_channel(window, cx);
            main.client.update(cx, |client, cx| client.join(cx));
        });
        (Screen::Welcome(welcome), subscription)
    }

    fn show(&mut self, screen: Screen, subscription: Option<Subscription>, window: &mut Window, cx: &mut Context<Self>) {
        (self.screen, self._screen) = (screen, subscription);
        // The fields of the screen before had the focus; the window's keys need it to go on.
        if !matches!(self.screen, Screen::Welcome(_)) {
            window.focus(&self.focus, cx);
        }
        self.watch(cx);
        cx.notify();
    }

    fn show_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (screen, subscription) = Self::channel(&self.client, self.updater.as_ref(), window, cx);
        self.show(screen, Some(subscription), window, cx);
    }

    fn show_welcome(&mut self, missing: Missing, window: &mut Window, cx: &mut Context<Self>) {
        let (screen, subscription) = Self::welcome(&self.client, missing, window, cx);
        self.show(screen, Some(subscription), window, cx);
    }

    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.client.update(cx, |client, cx| client.dismiss(cx));
        let settings = cx.new(|cx| Settings::new(self.client.clone(), window, cx));
        self.show(Screen::Settings(settings), None, window, cx);
    }

    /// Watches who is in the channel while the window shows it, and only then.
    fn watch(&self, cx: &mut Context<Self>) {
        let lobby = matches!(self.screen, Screen::Channel(_));
        self.client.update(cx, |client, _| client.set_lobby(lobby));
    }

    /// Takes the room key the keyring kept into the screen that shows it; asks for it if
    /// there is none.
    fn read_room_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Welcome(welcome) => welcome.update(cx, |welcome, cx| welcome.read_room_key(window, cx)),
            Screen::Settings(settings) => settings.update(cx, |settings, cx| settings.read_room_key(cx)),
            Screen::Channel(_) => {
                if let Some(missing) = self.client.read(cx).missing() {
                    self.show_welcome(missing, window, cx);
                }
            }
        }
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.client.read(cx).call.phase(), Phase::Failed(Failure::CertificateChanged { .. })) {
            self.client.update(cx, |client, cx| client.dismiss(cx));
            return;
        }
        match &self.screen {
            Screen::Settings(_) => self.show_channel(window, cx),
            Screen::Welcome(welcome) => {
                if !welcome.update(cx, |welcome, cx| welcome.back(window, cx)) {
                    cx.propagate();
                }
            }
            Screen::Channel(_) => cx.propagate(),
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
        let tokens = cx.tokens();
        let inset = kit::icon_inset(Size::Small);
        // Buttons cover the bar they sit on, which on Windows is the window's caption: else the
        // system takes their clicks for dragging the window.
        let content: Option<AnyElement> = match &self.screen {
            Screen::Channel(_) => Some(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(kit::wordmark())
                    .child(
                        div().occlude().child(
                            Button::new("settings", Variant::Ghost)
                                .size(Size::Small)
                                .square()
                                .icon(Glyph::Settings)
                                .tooltip("Einstellungen")
                                .on_click(cx.listener(|main, _, window, cx| main.show_settings(window, cx))),
                        ),
                    )
                    .into_any_element(),
            ),
            Screen::Settings(_) => Some(
                div()
                    .occlude()
                    .ml(-inset)
                    .child(
                        Button::new("back", Variant::Ghost)
                            .size(Size::Small)
                            .pl(inset)
                            .gap_1p5()
                            .text_color(tokens.text)
                            .icon(Glyph::Back)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Einstellungen"))
                            .on_click(cx.listener(|main, _, window, cx| main.show_channel(window, cx))),
                    )
                    .into_any_element(),
            ),
            Screen::Welcome(welcome) => (welcome.read(cx).step() == Missing::Place).then(|| {
                let welcome = welcome.clone();
                div()
                    .occlude()
                    .ml(-inset)
                    .child(Button::new("back", Variant::Ghost).size(Size::Small).square().icon(Glyph::Back).on_click(
                        move |_, window, cx| {
                            welcome.update(cx, |welcome, cx| welcome.back(window, cx));
                        },
                    ))
                    .into_any_element()
            }),
        };
        // The bar keeps its content off its left edge itself; a button's icon lines up on that
        // edge and as far off the right one, the button's surface reaching past it.
        TitleBar::new()
            .h(theme::TITLE_BAR)
            .bg(tokens.base)
            .border_0()
            .child(h_flex().w_full().h_full().pr(theme::TITLE_INSET - inset).children(content))
    }
}

impl Render for Main {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.screen {
            Screen::Channel(channel) => channel.clone().into_any_element(),
            Screen::Welcome(welcome) => welcome.clone().into_any_element(),
            Screen::Settings(settings) => settings.clone().into_any_element(),
        };
        let dialog = match *self.client.read(cx).call.phase() {
            Phase::Failed(Failure::CertificateChanged { pinned, presented, .. }) => {
                Some(certificate::dialog(&self.client, pinned, presented, cx))
            }
            _ => None,
        };
        let tokens = cx.tokens();
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(tokens.base)
            .text_color(tokens.text)
            .text_size(theme::TEXT)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::back))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .children(dialog)
    }
}
