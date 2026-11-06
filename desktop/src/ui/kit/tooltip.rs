//! A tooltip: a line of text floating beside the pointer while it rests on a control.

use gpui::{AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div};

use crate::ui::theme::{self, ActiveTokens, Typeset};

pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    /// What builds the tooltip showing `text`, as an element takes it.
    pub fn text(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView {
        move |_, cx| cx.new(|_| Self { text: text.clone() }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        // Off the pointer, which would otherwise cover its start; drawn above the window's
        // content, it takes nothing of the content's text style.
        div().pl_2().pt(theme::TOOLTIP_OFFSET).child(
            div()
                .px_2()
                .py_1()
                .rounded(theme::RADIUS_CONTROL_SM)
                .bg(tokens.floating)
                .border_1()
                .border_color(tokens.edge)
                .shadow(theme::depth::floating(tokens))
                .font_family(theme::SANS)
                .typeset(theme::TEXT_SM)
                .text_color(tokens.text)
                .child(self.text.clone()),
        )
    }
}
