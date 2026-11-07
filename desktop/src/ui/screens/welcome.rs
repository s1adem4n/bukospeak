//! The welcome, the first time and whenever there is no server to join: the server, and in.

use gpui::{AppContext, Context, Entity, EventEmitter, FontWeight, IntoElement, ParentElement, Render, Styled, Subscription, Window, div};

use crate::ui::client::Client;
use crate::ui::kit::{self, Button, Field, Size, TextField, TextFieldEvent, Variant, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

/// The welcome is through: the server is kept, and the window may join.
pub struct WelcomeEvent;

pub struct Welcome {
    client: Entity<Client>,
    server: Entity<TextField>,
    _events: Subscription,
}

impl EventEmitter<WelcomeEvent> for Welcome {}

impl Welcome {
    pub fn new(client: Entity<Client>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = client.read(cx).settings.server.clone();
        let server = cx.new(|cx| TextField::new(value, "server.example:9988", window, cx));
        let events = cx.subscribe_in(&server, window, |welcome, _, event: &TextFieldEvent, window, cx| match event {
            TextFieldEvent::Confirmed => welcome.finish(window, cx),
            // The button follows what is typed.
            TextFieldEvent::Changed => cx.notify(),
            TextFieldEvent::Left => {}
        });
        server.update(cx, |input, cx| input.focus(window, cx));
        Self { client, server, _events: events }
    }

    /// The server typed, trimmed.
    fn value(&self, cx: &Context<Self>) -> String {
        self.server.read(cx).value().trim().to_owned()
    }

    /// Keeps the server and is through, once there is one.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let server = self.value(cx);
        if server.is_empty() {
            self.server.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        self.client.update(cx, |client, cx| client.set_server(server, cx));
        cx.emit(WelcomeEvent);
    }
}

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.value(cx).is_empty();
        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .px_6()
            // The space above gives way first to a short window.
            .child(div().flex_1().max_h(theme::WELCOME_TOP))
            .child(
                v_flex()
                    .w_full()
                    .max_w(theme::WELCOME)
                    .items_center()
                    .child(kit::mark(theme::MARK))
                    .child(
                        div()
                            .mt_5()
                            .typeset(theme::TEXT_XL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_center()
                            .child("Willkommen bei BukoSpeak"),
                    )
                    .child(div().mt_1p5().text_center().text_color(cx.tokens().text_muted).child("Wo trefft ihr euch?"))
                    .child(
                        v_flex()
                            .w_full()
                            .mt_6()
                            .gap_4()
                            .child(Field::new(&self.server, Size::Large).font_family(theme::MONO).typeset(theme::TEXT))
                            .child(
                                Button::new("finish", Variant::Primary)
                                    .size(Size::Large)
                                    .w_full()
                                    .label("Beitreten")
                                    .disabled(empty)
                                    .on_click(cx.listener(|welcome, _, window, cx| welcome.finish(window, cx))),
                            ),
                    ),
            )
    }
}
