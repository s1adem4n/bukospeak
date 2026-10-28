//! The update window, at the start: it shows the updater looking for a newer release and
//! downloading and installing one it found, and opens the app's window in its place once there
//! is none, or once looking takes longer than the start should wait.

use std::time::Duration;

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable, Theme, v_flex};
use gpui_kit::{
    Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, Subscription,
    Task, Window, div,
};

use crate::ui::client::Client;
use crate::ui::kit;
use crate::ui::theme::{self, ActiveTokens};
use crate::update::{State, Updater};

/// How long the start waits for the updater to learn whether there is a newer release; one it
/// learns of later is installed once no call is up.
const PATIENCE: Duration = Duration::from_secs(3);

pub struct Update {
    client: Entity<Client>,
    updater: Entity<Updater>,
    /// Whether the app's window is open, this one closing.
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
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        let patience = cx.spawn_in(window, async move |update, cx| {
            cx.background_executor().timer(PATIENCE).await;
            // Gone with its window, when the app's is open already.
            let _ = update.update_in(cx, |update, window, cx| {
                if *update.updater.read(cx).state() == State::Checking {
                    update.open_app(window, cx);
                }
            });
        });
        Self { client, updater, done: false, _subscriptions: [observed, appearance], _patience: patience }
    }

    /// Opens the app's window in this one's place.
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            None => Spinner::new().color(tokens.text_faint).with_size(theme::ICON).into_any_element(),
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .bg(tokens.base)
            .text_color(tokens.text)
            .text_size(theme::TEXT)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(kit::mark(theme::MARK))
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(div().text_size(theme::TEXT_LG).font_weight(FontWeight::MEDIUM).child(headline))
                    .child(div().font_family(theme::MONO).text_size(theme::TEXT_SM).text_color(tokens.text_faint).child(versions)),
            )
            .child(v_flex().w(theme::UPDATE).h(theme::ICON).items_center().justify_center().child(progress))
    }
}
