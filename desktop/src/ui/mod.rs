mod chrome;
mod client;
mod glyph;
mod host;
mod kit;
pub mod marks;
mod screens;
mod text;
pub(crate) mod theme;

use gpui::{
    Animation, AnimationExt, App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, SharedString, Styled, Subscription, Window, WindowOptions, actions, div, ease_out_quint, size,
};

use crate::ui::chrome::TitleBar;
pub use crate::ui::client::Client;
pub use crate::ui::glyph::Assets;
use crate::ui::glyph::Glyph;
pub use crate::ui::host::{Desktop, Host};
use crate::ui::kit::{Button, Size, Variant, h_flex};
use crate::ui::screens::{Channel, ChannelEvent, Settings, Update, Welcome, WelcomeEvent};
use crate::ui::theme::{ActiveTokens, Typeset};
use crate::update::Updater;

actions!(bukospeak, [Back, FocusNext, FocusPrevious]);

const CONTEXT: &str = "Main";

pub fn init(cx: &mut App) {
    theme::install(cx);
    kit::init(cx);
    cx.bind_keys([
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
    ]);
}

pub fn start(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let Some(updater) = updater else {
        return open(client, None, cx);
    };
    let options = WindowOptions { is_minimizable: false, ..chrome::options(size(theme::UPDATE_WINDOW.0, theme::UPDATE_WINDOW.1), cx) };
    cx.open_window(options, |window, cx| {
        theme::follow(window, cx);
        cx.new(|cx| Update::new(client, updater, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

pub fn open(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut App) {
    let (width, height) = theme::WINDOW;
    cx.open_window(chrome::options(size(width, height), cx), |window, cx| {
        theme::follow(window, cx);
        cx.new(|cx| Main::new(client, updater, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    Unreadable,
    NotSaved,
    NvidiaUnavailable,
    NvidiaNotInstalled,
    NvidiaFailed,
    StartNotSet,
}

enum Screen {
    Channel(Entity<Channel>),
    Welcome(Entity<Welcome>),
    Settings(Entity<Settings>),
}

pub struct Main {
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    screen: Screen,
    focus: FocusHandle,
    _screen: Option<Subscription>,
    _subscriptions: [Subscription; 2],
}

impl Main {
    pub(crate) fn new(client: Entity<Client>, updater: Option<Entity<Updater>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |_, _, cx| cx.notify());
        let appearance = cx.observe_window_appearance(window, |_, window, cx| theme::follow(window, cx));
        cx.on_release(|main, cx| {
            main.client.update(cx, |client, _| {
                client.set_lobby(false);
                client.set_listening(false);
            })
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let (screen, subscription) = match client.read(cx).settings.is_complete() {
            true => Self::channel(&client, updater.as_ref(), window, cx),
            false => Self::welcome(&client, window, cx),
        };
        let main = Self { client, updater, screen, focus, _screen: Some(subscription), _subscriptions: [observed, appearance] };
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
        let subscription = cx.subscribe_in(&channel, window, |main, _, _: &ChannelEvent, window, cx| main.show_welcome(window, cx));
        (Screen::Channel(channel), subscription)
    }

    fn welcome(client: &Entity<Client>, window: &mut Window, cx: &mut Context<Self>) -> (Screen, Subscription) {
        let welcome = cx.new(|cx| Welcome::new(client.clone(), window, cx));
        let subscription = cx.subscribe_in(&welcome, window, |main, _, _: &WelcomeEvent, window, cx| {
            main.show_channel(window, cx);
            main.client.update(cx, |client, cx| client.join(cx));
        });
        (Screen::Welcome(welcome), subscription)
    }

    fn show(&mut self, screen: Screen, subscription: Option<Subscription>, window: &mut Window, cx: &mut Context<Self>) {
        (self.screen, self._screen) = (screen, subscription);
        // The previous screen's fields had the focus; the window's keys need it back.
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

    fn show_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (screen, subscription) = Self::welcome(&self.client, window, cx);
        self.show(screen, Some(subscription), window, cx);
    }

    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.client.update(cx, |client, cx| client.dismiss(cx));
        let settings = cx.new(|cx| Settings::new(self.client.clone(), window, cx));
        self.show(Screen::Settings(settings), None, window, cx);
    }

    fn watch(&self, cx: &mut Context<Self>) {
        let (lobby, listening) = (matches!(self.screen, Screen::Channel(_)), matches!(self.screen, Screen::Settings(_)));
        self.client.update(cx, |client, _| {
            client.set_lobby(lobby);
            client.set_listening(listening);
        });
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Settings(_) => self.show_channel(window, cx),
            Screen::Welcome(_) | Screen::Channel(_) => cx.propagate(),
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
        let (title, open) = match &self.screen {
            Screen::Channel(_) => (SharedString::from(host(&self.client.read(cx).settings.server).to_owned()), false),
            Screen::Settings(_) => ("Einstellungen".into(), true),
            Screen::Welcome(_) => return TitleBar::new(None),
        };
        let content = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .justify_between()
            .gap(theme::GAP)
            // The icon lines up on the bar's inset; its surface reaches past it.
            .mr(-kit::icon_inset(Size::Small))
            .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).text_color(tokens.text_muted).child(title))
            .child(
                // Buttons cover the bar they sit on, which on Windows is the window's caption:
                // else the system takes their clicks for dragging the window.
                div().occlude().child(
                    Button::new("settings", Variant::Ghost)
                        .size(Size::Small)
                        .square()
                        .icon(Glyph::Settings)
                        .selected(open)
                        .name("Einstellungen")
                        .on_click(cx.listener(move |main, _, window, cx| match open {
                            true => main.show_channel(window, cx),
                            false => main.show_settings(window, cx),
                        })),
                ),
            );
        TitleBar::new(Some(content.into_any_element()))
    }
}

impl Render for Main {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (content, screen, enters_from) = match &self.screen {
            Screen::Channel(channel) => (channel.clone().into_any_element(), 0_usize, -1.),
            Screen::Welcome(welcome) => (welcome.clone().into_any_element(), 1, 0.),
            Screen::Settings(settings) => (settings.clone().into_any_element(), 2, 1.),
        };
        let content = div().relative().flex_1().min_h_0().flex().flex_col().child(content).with_animation(
            ("screen", screen),
            Animation::new(theme::APPEAR).with_easing(ease_out_quint()),
            move |content, delta| content.opacity(delta).left(theme::RISE * enters_from * (1. - delta)),
        );
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
            .child(content);
        chrome::frame(chrome::rounded(main, window), window, cx)
    }
}

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
