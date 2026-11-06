//! The welcome, the first time and whenever something is missing to join: the name the others
//! see, then the server and its room key, and in.

use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, ease_out_quint,
};

use crate::ui::client::{Client, Missing};
use crate::ui::kit::{self, Avatar, Button, Field, Size, TextField, TextFieldEvent, Variant, h_flex, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

/// The welcome is through: what it asked for is kept, and the window may join.
pub struct WelcomeEvent;

pub struct Welcome {
    client: Entity<Client>,
    step: Missing,
    name: Entity<TextField>,
    server: Entity<TextField>,
    room_key: Entity<TextField>,
    _events: [Subscription; 3],
}

impl EventEmitter<WelcomeEvent> for Welcome {}

impl Welcome {
    pub fn new(client: Entity<Client>, step: Missing, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let read = client.read(cx);
        let values = [read.settings.name.clone(), read.settings.server.clone(), read.room_key().to_owned()];
        let [name, server, room_key] = values;
        let mut input = |value: String, placeholder: &'static str, secret: bool| {
            cx.new(|cx| {
                let field = TextField::new(value, placeholder, window, cx);
                if secret { field.secret() } else { field }
            })
        };
        let (name, server, room_key) =
            (input(name, "Name", false), input(server, "server.example:9988", false), input(room_key, "Raumschlüssel", true));
        let enter = |input: &Entity<TextField>, then: fn(&mut Self, &mut Window, &mut Context<Self>), cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, move |welcome, _, event: &TextFieldEvent, window, cx| match event {
                TextFieldEvent::Confirmed => then(welcome, window, cx),
                // The buttons and the avatar follow what is typed.
                TextFieldEvent::Changed => cx.notify(),
                TextFieldEvent::Left => {}
            })
        };
        let events = [enter(&name, Self::next, cx), enter(&server, Self::after_server, cx), enter(&room_key, Self::finish, cx)];
        let welcome = Self { client, step, name, server, room_key, _events: events };
        welcome.focus_empty(window, cx);
        welcome
    }

    pub fn step(&self) -> Missing {
        self.step
    }

    /// Takes the room key the keyring kept into its field while that is empty.
    pub fn read_room_key(&mut self, cx: &mut Context<Self>) {
        if value(&self.room_key, cx).is_empty() {
            let key = self.client.read(cx).room_key().to_owned();
            self.room_key.update(cx, |input, cx| input.set_value(key, cx));
        }
    }

    /// The fields of the step, in order.
    fn fields(&self) -> Vec<&Entity<TextField>> {
        match self.step {
            Missing::Name => vec![&self.name],
            Missing::Place => vec![&self.server, &self.room_key],
        }
    }

    /// Focuses the step's first field still empty, else its first; true if one was empty.
    fn focus_empty(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let fields = self.fields();
        let empty = fields.iter().find(|field| value(field, cx).is_empty()).copied();
        let field = empty.unwrap_or(fields[0]);
        field.update(cx, |input, cx| input.focus(window, cx));
        empty.is_some()
    }

    /// From the name on to the place, once there is a name.
    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.step == Missing::Name && !value(&self.name, cx).is_empty() {
            self.step = Missing::Place;
            self.focus_empty(window, cx);
            cx.notify();
        }
    }

    /// Back from the place to the name; false if the welcome is at its first step.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.step == Missing::Name {
            return false;
        }
        self.step = Missing::Name;
        self.focus_empty(window, cx);
        cx.notify();
        true
    }

    /// Enter on the server goes on to the room key while it is empty.
    fn after_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if value(&self.room_key, cx).is_empty() {
            self.room_key.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.finish(window, cx);
        }
    }

    /// Keeps what the welcome was told and is through, once all of it is there.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_empty(window, cx) {
            return;
        }
        let [name, server, room_key] = [&self.name, &self.server, &self.room_key].map(|input| value(input, cx));
        self.client.update(cx, |client, cx| {
            client.set_name(name, cx);
            client.set_server(server, cx);
            client.set_room_key(room_key, cx);
        });
        cx.emit(WelcomeEvent);
    }

    /// Where in the welcome one is, a dot for each step; a click on one goes there, to the
    /// place once there is a name.
    fn render_steps(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let (current, other, hovered) = (tokens.text_muted, tokens.text_faint, tokens.text_muted);
        let step = self.step;
        let dot = |shown: Missing, cx: &mut Context<Self>| {
            let (width, color) = if shown == step { (theme::STEP_DOT_CURRENT, current) } else { (theme::STEP_DOT, other) };
            // The dot is small; the space around it takes the click too.
            div()
                .id(("step", shown as usize))
                .group("step")
                .p_1()
                .cursor_pointer()
                .child(
                    div()
                        .h(theme::STEP_DOT)
                        .w(width)
                        .rounded_full()
                        .bg(color)
                        .when(shown != step, |dot| dot.group_hover("step", |dot| dot.bg(hovered))),
                )
                .on_click(cx.listener(move |welcome, _, window, cx| match shown {
                    Missing::Name => {
                        welcome.back(window, cx);
                    }
                    Missing::Place => welcome.next(window, cx),
                }))
        };
        h_flex().absolute().bottom_5().gap_0p5().child(dot(Missing::Name, cx)).child(dot(Missing::Place, cx))
    }
}

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = value(&self.name, cx);
        let content = match self.step {
            Missing::Name => v_flex()
                .w_full()
                .items_center()
                .child(kit::mark(theme::MARK))
                .child(headline("Willkommen bei BukoSpeak"))
                .child(subline("Wie sollen dich die anderen nennen?", cx))
                .child(form(
                    [Field::new(&self.name, Size::Large).into_any_element()],
                    Button::new("next", Variant::Primary)
                        .label("Weiter")
                        .disabled(name.is_empty())
                        .on_click(cx.listener(|welcome, _, window, cx| welcome.next(window, cx))),
                )),
            Missing::Place => {
                let complete = [&self.server, &self.room_key].iter().all(|field| !value(field, cx).is_empty());
                v_flex()
                    .w_full()
                    .items_center()
                    // As high as the mark it takes the place of, so the step's lines stay where they were.
                    .child(Avatar::new(name.clone(), theme::AVATAR_XL))
                    .child(headline(format!("Hallo, {name}")))
                    .child(subline("Wo trefft ihr euch?", cx))
                    .child(form(
                        [
                            Field::new(&self.server, Size::Large).font_family(theme::MONO).typeset(theme::TEXT).into_any_element(),
                            Field::new(&self.room_key, Size::Large).into_any_element(),
                        ],
                        Button::new("finish", Variant::Primary)
                            .label("Beitreten")
                            .disabled(!complete)
                            .on_click(cx.listener(|welcome, _, window, cx| welcome.finish(window, cx))),
                    ))
            }
        };
        // Forward slides in from the right, back from the left.
        let from = match self.step {
            Missing::Name => -1.,
            Missing::Place => 1.,
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .relative()
            .items_center()
            .px_6()
            // The column hangs from the same height in every step, so going on moves nothing but
            // what changes; the space above gives way first to a short window.
            .child(div().flex_1().max_h(theme::WELCOME_TOP))
            .child(div().w_full().max_w(theme::WELCOME).relative().child(content).with_animation(
                ("step", self.step as usize),
                Animation::new(theme::APPEAR).with_easing(ease_out_quint()),
                move |content, delta| content.left(theme::RISE * from * (1. - delta)).opacity(delta),
            ))
            .child(self.render_steps(cx))
    }
}

/// A step's fields, close together, and the button that takes them, further off.
fn form(fields: impl IntoIterator<Item = AnyElement>, button: Button) -> impl IntoElement {
    v_flex().w_full().mt_6().gap_4().child(v_flex().gap_2().children(fields)).child(button.size(Size::Large).w_full())
}

fn value(input: &Entity<TextField>, cx: &App) -> String {
    input.read(cx).value().trim().to_owned()
}

fn headline(text: impl Into<SharedString>) -> impl IntoElement {
    div().mt_5().typeset(theme::TEXT_XL).font_weight(FontWeight::SEMIBOLD).text_center().child(text.into())
}

fn subline(text: &'static str, cx: &App) -> impl IntoElement {
    div().mt_1p5().text_center().text_color(cx.tokens().text_muted).child(text)
}
