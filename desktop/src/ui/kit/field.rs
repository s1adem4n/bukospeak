//! Text fields in the app's looks: [`Field`], always open for typing, and [`InlineField`], which
//! shows its value until clicked and keeps what was typed once Enter is pressed or the focus goes.

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Refineable, Render, RenderOnce, SharedString, StyleRefinement, Styled, Subscription, Window, actions, div,
};

use crate::ui::kit::{Button, Size, TextField, TextFieldEvent, Variant, h_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

actions!(
    inline_field,
    [
        /// Escape while editing: the field goes back to what it held.
        Cancel
    ]
);

const CONTEXT: &str = "InlineField";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
}

/// A text field as large as a button of its size, set into the panel it is on; its edge lights
/// up while it has the focus. Its text takes the text style it is given.
#[derive(IntoElement)]
pub struct Field {
    field: Entity<TextField>,
    size: Size,
    style: StyleRefinement,
}

impl Field {
    pub fn new(field: &Entity<TextField>, size: Size) -> Self {
        Self { field: field.clone(), size, style: StyleRefinement::default() }
    }
}

impl Styled for Field {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Field {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let focused = self.field.read(cx).focus_handle(cx).contains_focused(window, cx);
        let metrics = self.size.metrics();
        let mut field = div()
            .flex_none()
            .w_full()
            .h(metrics.height)
            .rounded(metrics.radius)
            .bg(tokens.inset)
            .border_1()
            .border_color(if focused { tokens.focus } else { tokens.hairline })
            .typeset(metrics.text)
            .text_color(tokens.text);
        field.style().refine(&self.style);
        field.child(self.field)
    }
}

/// How an inline field shows its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// At a row's end, quieter than the row's label: a name.
    Plain,
    /// At a row's end, quieter than the row's label, in the mono font: a server.
    Mono,
    /// At a row's end, but dots: a key, which the field shows while typing only on request.
    Secret,
}

/// What an inline field holds changed: an edit ended with another value than before.
pub struct InlineFieldEvent(pub String);

/// A value shown as text until clicked, then edited in place: Enter or the focus going keeps
/// what was typed, Escape goes back.
pub struct InlineField {
    field: Entity<TextField>,
    look: Look,
    placeholder: SharedString,
    /// The value outside an edit: kept, shown, and gone back to on Escape.
    value: String,
    /// Whether an empty value is refused, the field going back instead.
    required: bool,
    editing: bool,
    /// Where the focus goes after an edit, so the keys go on from here.
    focus: FocusHandle,
    _events: Subscription,
}

impl EventEmitter<InlineFieldEvent> for InlineField {}

impl InlineField {
    pub fn new(value: String, placeholder: &'static str, look: Look, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let field = TextField::new(String::new(), placeholder, window, cx);
            if look == Look::Secret { field.secret() } else { field }
        });
        let events = cx.subscribe_in(&field, window, |inline, _, event: &TextFieldEvent, window, cx| match event {
            TextFieldEvent::Confirmed | TextFieldEvent::Left => inline.keep(window, cx),
            TextFieldEvent::Changed => {}
        });
        Self {
            field,
            look,
            placeholder: placeholder.into(),
            value,
            required: false,
            editing: false,
            focus: cx.focus_handle().tab_stop(true),
            _events: events,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Shows `value`, unless the user is editing.
    pub fn set_value(&mut self, value: String, cx: &mut Context<Self>) {
        if !self.editing {
            self.value = value;
            cx.notify();
        }
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = true;
        let value = self.value.clone();
        self.field.update(cx, |field, cx| {
            field.set_value(value, cx);
            field.select_all(cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    fn keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        self.end(window, cx);
        let typed = self.field.read(cx).value().trim().to_owned();
        if typed != self.value && !(self.required && typed.is_empty()) {
            self.value = typed.clone();
            cx.emit(InlineFieldEvent(typed));
        }
    }

    /// Ends an edit without keeping it; outside an edit, Escape goes on to the window.
    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing {
            self.end(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn render_value(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let empty = self.value.is_empty();
        let text: SharedString = match (self.look, empty) {
            (_, true) => self.placeholder.clone(),
            (Look::Secret, false) => "••••••••".into(),
            (_, false) => self.value.clone().into(),
        };
        Button::new("value", Variant::Bare)
            .track_focus(&self.focus)
            .w_full()
            .h(theme::ROW)
            .justify_end()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::NORMAL)
                    // A value takes the button's color, which brightens on hover.
                    .when(empty, |text| text.text_color(tokens.text_faint))
                    .when(self.look == Look::Mono, |text| text.font_family(theme::MONO).typeset(theme::TEXT_SM))
                    .child(text),
            )
            .on_click(cx.listener(|inline, _, window, cx| inline.edit(window, cx)))
    }
}

impl Render for InlineField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.editing {
            Field::new(&self.field, Size::Small)
                .when(self.look == Look::Mono, |field| field.font_family(theme::MONO).typeset(theme::TEXT_SM))
                .into_any_element()
        } else {
            self.render_value(cx).into_any_element()
        };
        h_flex().key_context(CONTEXT).on_action(cx.listener(Self::cancel)).flex_1().min_w_0().child(content)
    }
}
