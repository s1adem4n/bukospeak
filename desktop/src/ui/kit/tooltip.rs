//! A tooltip: a line of text floating beside the pointer while it rests on a control.

use gpui::{AnyView, App, AppContext, BoxShadow, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px};

use crate::ui::theme::{self, ActiveTokens};

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
        div().pl_2().pt_2p5().child(
            div()
                .px_2()
                .py_1()
                .rounded(theme::RADIUS_SMALL)
                .bg(tokens.floating)
                .border_1()
                .border_color(tokens.edge)
                .shadow(vec![BoxShadow::new(px(0.), px(2.), tokens.shadow).blur_radius(px(6.))])
                .font_family(theme::SANS)
                .text_size(theme::TEXT_SM)
                .text_color(tokens.text)
                .child(self.text.clone()),
        )
    }
}
