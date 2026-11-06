//! The window: a welcome the first time, then the channel with who is in it and the dock to
//! join, mute and leave; the settings; and the dialog for a changed certificate. Before it, at
//! the start, the update window, while a newer release is looked for.

mod chrome;
mod client;
mod glyph;
mod host;
mod kit;
pub mod marks;
mod screens;
mod text;
pub(crate) mod theme;

use crate::call::{Failure, Phase};
use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, Styled, Subscription, Window, WindowOptions, actions, div, ease_out_quint, size,
};

use crate::ui::chrome::TitleBar;
use crate::ui::client::Missing;
pub use crate::ui::client::{Client, RoomKeyRead};
pub use crate::ui::glyph::Assets;
use crate::ui::glyph::Glyph;
pub use crate::ui::host::{Desktop, Host};
use crate::ui::kit::{Button, Size, Variant, h_flex};
use crate::ui::screens::{Channel, ChannelEvent, Settings, Update, Welcome, WelcomeEvent, certificate};
use crate::ui::theme::{ActiveTokens, Typeset};
use crate::update::Updater;

actions!(
    bukospeak,
    [
        /// Escape: closes the dialog or the settings, or goes a step back.
        Back,
        /// Tab: the focus on to the next control.
        FocusNext,
        /// Shift-Tab: back to the one before.
        FocusPrevious
    ]
);

/// The key context of the window's content, which Back is bound in.
const CONTEXT: &str = "Main";

/// Installs the looks and binds the keys, once, before a window opens.
pub fn init(cx: &mut App) {
    theme::install(cx);
    kit::init(cx);
    cx.bind_keys([
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
    ]);
}

/// Opens the window at the start: with an updater, the update window first, which opens it.
pub fn start(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let Some(updater) = updater else {
        return open(client, None, cx);
    };
    // No title bar: the window is moved by any of it.
    let options = WindowOptions {
        is_resizable: false,
        is_minimizable: false,
        ..chrome::options(size(theme::UPDATE_WINDOW.0, theme::UPDATE_WINDOW.1), cx)
    };
    cx.open_window(options, |window, cx| {
        theme::follow(window, cx);
        cx.new(|cx| Update::new(client, updater, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

/// Opens the window, which shows the client and a release the updater waits to install.
pub fn open(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let (width, height) = theme::WINDOW;
    let (least_width, least_height) = theme::WINDOW_MIN;
    let options = WindowOptions { window_min_size: Some(size(least_width, least_height)), ..chrome::options(size(width, height), cx) };
    cx.open_window(options, |window, cx| {
        theme::follow(window, cx);
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
    /// The server offers no NVIDIA effects for the card: Standard suppresses the noise.
    NvidiaUnavailable,
    /// NVIDIA's effects didn't install: Standard suppresses the noise.
    NvidiaNotInstalled,
    /// NVIDIA's effect didn't start: Standard suppresses the noise instead.
    NvidiaFailed,
    /// The system wasn't told to start the app with it, or not to any more.
    StartNotSet,
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
    pub(crate) fn new(client: Entity<Client>, updater: Option<Entity<Updater>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |_, _, cx| cx.notify());
        let read = cx.subscribe_in(&client, window, |main, _, _: &RoomKeyRead, window, cx| main.read_room_key(window, cx));
        let appearance = cx.observe_window_appearance(window, |_, window, cx| theme::follow(window, cx));
        // A closed window no longer shows who is in the channel, nor the microphone's level.
        cx.on_release(|main, cx| {
            main.client.update(cx, |client, _| {
                client.set_lobby(false);
                client.set_listening(false);
            })
        })
        .detach();
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

    /// Watches who is in the channel while the window shows it, and listens to the microphone
    /// while the settings show its level; only then.
    fn watch(&self, cx: &mut Context<Self>) {
        let (lobby, listening) = (matches!(self.screen, Screen::Channel(_)), matches!(self.screen, Screen::Settings(_)));
        self.client.update(cx, |client, _| {
            client.set_lobby(lobby);
            client.set_listening(listening);
        });
    }

    /// Takes the room key the keyring kept into the screen that shows it; asks for it if
    /// there is none.
    fn read_room_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Welcome(welcome) => welcome.update(cx, |welcome, cx| welcome.read_room_key(cx)),
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

    fn focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn focus_previous(&mut self, _: &FocusPrevious, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
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
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tokens.text_muted)
                            .child(host(&self.client.read(cx).settings.server).to_owned()),
                    )
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
                    // The chevron's stroke, not its blank, lines up with the panels' edges.
                    .ml(-inset - theme::BACK_BEARING)
                    .child(
                        Button::new("back", Variant::Ghost)
                            .size(Size::Small)
                            .pl(inset)
                            // The title starts where the settings' titles and labels do.
                            .gap(theme::PANEL_INSET - theme::ICON + theme::BACK_BEARING)
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
                    // The chevron's stroke, not its blank, lines up with the panels' edges.
                    .ml(-inset - theme::BACK_BEARING)
                    .child(Button::new("back", Variant::Ghost).size(Size::Small).square().icon(Glyph::Back).on_click(
                        move |_, window, cx| {
                            welcome.update(cx, |welcome, cx| welcome.back(window, cx));
                        },
                    ))
                    .into_any_element()
            }),
        };
        // A button's icon lines up on the bar's inset, the button's surface reaching past it.
        TitleBar::new(content.map(|content| h_flex().flex_1().min_w_0().h_full().mr(-inset).child(content).into_any_element()))
    }
}

impl Render for Main {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A screen comes on from the side it lies on: the settings beyond the channel.
        let (content, screen, from) = match &self.screen {
            Screen::Channel(channel) => (channel.clone().into_any_element(), 0_usize, -1.),
            Screen::Welcome(welcome) => (welcome.clone().into_any_element(), 1, 0.),
            Screen::Settings(settings) => (settings.clone().into_any_element(), 2, 1.),
        };
        let content = div().relative().flex_1().min_h_0().flex().flex_col().child(content).with_animation(
            ("screen", screen),
            Animation::new(theme::APPEAR).with_easing(ease_out_quint()),
            move |content, delta| content.opacity(delta).left(theme::RISE * from * (1. - delta)),
        );
        let dialog = match *self.client.read(cx).call.phase() {
            Phase::Failed(Failure::CertificateChanged { pinned, presented, .. }) => {
                Some(certificate::dialog(&self.client, pinned, presented, window, cx))
            }
            _ => None,
        };
        let tokens = cx.tokens();
        let main = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(tokens.base)
            .font_family(theme::SANS)
            .text_color(tokens.text)
            .typeset(theme::TEXT)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .child(self.render_title_bar(cx))
            .child(content)
            .children(dialog);
        chrome::frame(chrome::rounded(main, window), window, cx)
    }
}

/// The host of a server's address, without its port: what the title bar names it by.
fn host(server: &str) -> &str {
    match server.rsplit_once(':') {
        // An IPv6 address alone has colons too, but no port after a bracket.
        Some((host, port)) if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) && !host.ends_with(':') => {
            host.trim_start_matches('[').trim_end_matches(']')
        }
        _ => server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_bar_names_a_server_by_its_host() {
        assert_eq!(host("voice.example:9988"), "voice.example");
        assert_eq!(host("voice.example"), "voice.example");
        assert_eq!(host("[::1]:9988"), "::1");
        assert_eq!(host("10.0.0.2:9988"), "10.0.0.2");
    }
}
