use std::time::Duration;

use gpui::{
    Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, Subscription,
    Task, Window, div,
};

use crate::ui::chrome;
use crate::ui::client::Client;
use crate::ui::kit::{self, Spinner, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};
use crate::update::{State, Updater};

const MAX_CHECK_WAIT: Duration = Duration::from_secs(3);

pub struct Update {
    client: Entity<Client>,
    updater: Entity<Updater>,
    done: bool,
    _subscriptions: [Subscription; 2],
    _patience: Task<()>,
}

impl Update {
    pub fn new(client: Entity<Client>, updater: Entity<Updater>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe_in(&updater, window, |update, updater, window, cx| {
            if *updater.read(cx).state() == State::Current {
                update.open_app(window, cx);
            }
            cx.notify();
        });
        let appearance = cx.observe_window_appearance(window, |_, window, cx| theme::follow(window, cx));
        let patience = cx.spawn_in(window, async move |update, cx| {
            cx.background_executor().timer(MAX_CHECK_WAIT).await;
            let _ = update.update_in(cx, |update, window, cx| {
                if *update.updater.read(cx).state() == State::Checking {
                    update.open_app(window, cx);
                }
            });
        });
        Self { client, updater, done: false, _subscriptions: [observed, appearance], _patience: patience }
    }

    fn open_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        crate::ui::open(self.client.clone(), Some(self.updater.clone()), cx);
        window.remove_window();
    }
}

impl Render for Update {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.read(cx);
        let current = updater.current();
        let (headline, version, done): (&str, Option<&str>, Option<f32>) = match updater.state() {
            State::Checking | State::Current => ("Suche nach Updates…", None, None),
            State::Downloading { version, percent } => ("Lade Update…", Some(version), Some(f32::from(*percent) / 100.)),
            State::Ready { version } | State::Installing { version } => ("Starte neu…", Some(version), Some(1.)),
        };
        let versions: SharedString = match version {
            Some(version) => format!("{current} → {version}").into(),
            None => current.to_owned().into(),
        };
        let tokens = cx.tokens();
        let progress = match done {
            Some(done) => kit::progress(done, cx).into_any_element(),
            None => Spinner::new(theme::ICON).color(tokens.text_faint).into_any_element(),
        };
        let content = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(theme::SECTION)
            .bg(tokens.base)
            .font_family(theme::SANS)
            .text_color(tokens.text)
            .typeset(theme::TEXT)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(kit::mark(theme::MARK))
            .child(
                v_flex()
                    .items_center()
                    .gap(theme::LINE)
                    .child(div().typeset(theme::TEXT_MD).font_weight(FontWeight::MEDIUM).child(headline))
                    .child(div().font_family(theme::MONO).typeset(theme::TEXT_SM).text_color(tokens.text_faint).child(versions)),
            )
            .child(v_flex().w(theme::UPDATE).h(theme::ICON).items_center().justify_center().child(progress));
        chrome::frame(chrome::rounded(content, window), window, cx)
    }
}
