//! The settings: groups of rows, a label and its field each, kept when they close.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
};

use crate::ui::{Form, Main, edit_menu, theme};

impl Main {
    pub(super) fn render_settings(&self, form: &Form, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .w_full()
                .max_w(theme::SETTINGS)
                .mx_auto()
                .px_4()
                .pt_2()
                .pb_6()
                .gap_6()
                .child(group("Profil", [("Name", &form.name)], cx))
                .child(group("Server", [("Adresse", &form.server), ("Raumschlüssel", &form.room_key)], cx)),
        )
    }
}

/// A titled card of rows.
fn group<const N: usize>(title: &'static str, rows: [(&'static str, &Entity<InputState>); N], cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap_2()
        .child(div().px_1().text_size(theme::TEXT_SM).font_weight(FontWeight::MEDIUM).text_color(theme.muted_foreground).child(title))
        .child(v_flex().rounded(theme.radius_lg).border_1().border_color(theme.border).bg(theme.muted).children(
            rows.into_iter().enumerate().map(|(index, (label, input))| {
                h_flex()
                    .h(px(44.))
                    .pl_3()
                    .pr_1()
                    .gap_3()
                    .when(index > 0, |row| row.border_t_1().border_color(theme.border))
                    .child(div().w(px(104.)).flex_none().child(label))
                    .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).large().context_menu(edit_menu)))
            }),
        ))
}
