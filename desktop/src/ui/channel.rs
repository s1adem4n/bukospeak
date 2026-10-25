//! The channel: who is in it, from outside or in the call, and below the bar that tells how the
//! call is, with the button to join or, in it, the buttons to mute and leave.

use bukospeak_desktop::call::Phase;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
};

use crate::ui::glyph::Glyph;
use crate::ui::member::Row;
use crate::ui::{Main, Tone, control, primary, text, theme};

impl Main {
    pub(super) fn render_channel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<Row> = if self.call.is_in_call() {
            let you = self.call.you();
            let members = self.call.members().iter();
            members
                .map(|member| {
                    let yours = Some(member.id) == you;
                    // One's own as set here, before the server tells it back.
                    let muted = if yours { self.call.is_muted() } else { member.muted };
                    let speaks = self.call.speaking().contains(&member.id);
                    Row::new(member.id.0, member.name.clone(), speaks, muted, member.reconnecting).yours(yours)
                })
                .collect()
        } else {
            let present = self.call.present().iter();
            present.map(|member| Row::new(member.id.0, member.name.clone(), false, member.muted, member.reconnecting)).collect()
        };
        let list = if rows.is_empty() { self.render_empty(cx) } else { self.render_list(rows) };
        let message = self.render_message(cx).map(|message| div().px(theme::GUTTER).pb_2().child(message));
        v_flex().flex_1().min_h_0().child(list).children(message).child(self.render_bar(cx))
    }

    fn render_list(&self, rows: Vec<Row>) -> AnyElement {
        div()
            .id("members")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(theme::GUTTER - theme::ROW_INSET)
            .py_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .when(*self.call.phase() == Phase::Reconnecting, |list| list.opacity(theme::RECONNECTING_OPACITY))
            .children(rows)
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(theme.muted_foreground)
            .child(Icon::from(Glyph::Users).size(px(20.)))
            .child("Niemand im Kanal")
            .into_any_element()
    }

    /// How the call is, on the left; on the right Beitreten, or in the call mute and leave.
    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (status, color) = match self.call.phase() {
            Phase::Idle => ("Nicht verbunden", theme.muted_foreground),
            Phase::Joining => ("Verbinde…", theme.muted_foreground),
            Phase::Connected => ("Verbunden", theme::speaking(theme.is_dark())),
            Phase::Reconnecting => ("Verbinde neu…", theme.warning),
            Phase::Failed(failure) => (text::failure(failure), theme.danger),
        };
        let (surface, quiet) = (theme.muted, theme.muted_foreground);
        let actions: Vec<AnyElement> = if self.call.is_in_call() {
            let mute = if self.call.is_muted() {
                control("mute", Glyph::MicOff, "Stummschaltung aufheben", Tone::Off, cx)
            } else {
                control("mute", Glyph::Mic, "Stummschalten", Tone::Plain, cx)
            };
            vec![
                mute.on_click(cx.listener(|main, _, _, cx| main.toggle_mute(cx))).into_any_element(),
                control("leave", Glyph::PhoneOff, "Verlassen", Tone::End, cx)
                    .on_click(cx.listener(|main, _, _, cx| main.leave(cx)))
                    .into_any_element(),
            ]
        } else {
            let joining = *self.call.phase() == Phase::Joining;
            vec![
                primary("join", "Beitreten")
                    .h(theme::CONTROL)
                    .px_3p5()
                    .loading(joining)
                    .on_click(cx.listener(|main, _, window, cx| {
                        if *main.call.phase() != Phase::Joining {
                            main.join(window, cx);
                        }
                    }))
                    .into_any_element(),
            ]
        };
        h_flex()
            .flex_none()
            .mx(theme::GUTTER - theme::ROW_INSET)
            .mb(theme::GUTTER - theme::ROW_INSET)
            .p_1p5()
            .pl(theme::ROW_INSET + px(4.))
            .gap_1()
            .rounded(theme.radius_lg)
            .bg(surface)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_size(theme::TEXT_SM)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(div().flex_none().size_1p5().rounded_full().bg(color))
                            .child(div().truncate().child(status)),
                    )
                    .child(div().text_size(theme::TEXT_XS).text_color(quiet).truncate().child(self.settings.server.clone())),
            )
            .children(actions)
    }
}
