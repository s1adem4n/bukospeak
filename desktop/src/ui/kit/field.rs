use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Refineable, Render, RenderOnce, SharedString, StyleRefinement, Styled, Subscription, Window, actions, div,
};

use crate::ui::kit::{Button, Size, TextField, TextFieldEvent, Variant, h_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

actions!(inline_field, [Cancel]);

const CONTEXT: &str = "InlineField";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
}

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

pub struct InlineFieldEvent(pub String);

pub struct InlineField {
    field: Entity<TextField>,
    placeholder: SharedString,
    value: String,
    editing: bool,
    focus: FocusHandle,
    _events: Subscription,
}

impl EventEmitter<InlineFieldEvent> for InlineField {}

impl InlineField {
    pub fn new(value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| TextField::new(String::new(), placeholder, window, cx));
        let events = cx.subscribe_in(&field, window, |inline, _, event: &TextFieldEvent, window, cx| match event {
            TextFieldEvent::Confirmed | TextFieldEvent::Left => inline.keep(window, cx),
            TextFieldEvent::Changed => {}
        });
        Self { field, placeholder: placeholder.into(), value, editing: false, focus: cx.focus_handle().tab_stop(true), _events: events }
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
        if typed != self.value {
            self.value = typed.clone();
            cx.emit(InlineFieldEvent(typed));
        }
    }

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
        let text: SharedString = if empty { self.placeholder.clone() } else { self.value.clone().into() };
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
                    .font_family(theme::MONO)
                    .typeset(theme::TEXT_SM)
                    .child(text),
            )
            .on_click(cx.listener(|inline, _, window, cx| inline.edit(window, cx)))
    }
}

impl Render for InlineField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.editing {
            Field::new(&self.field, Size::Small).font_family(theme::MONO).typeset(theme::TEXT_SM).into_any_element()
        } else {
            self.render_value(cx).into_any_element()
        };
        h_flex().key_context(CONTEXT).on_action(cx.listener(Self::cancel)).flex_1().min_w_0().child(content)
    }
}
