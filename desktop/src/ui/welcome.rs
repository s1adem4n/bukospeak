//! The welcome, the first time: the name the others see, then the server and its room key, and
//! in.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Animation, AnimationExt, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ObjectFit, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, div, img, px,
};

use crate::ui::member::Avatar;
use crate::ui::{Field, Form, Main, Step, edit_menu, primary, theme};

impl Main {
    pub(super) fn render_welcome(&self, form: &Form, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let name = form.value(Field::Name, cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let content = match step {
            Step::Name => v_flex()
                .w_full()
                .items_center()
                .child(img(self.mark.clone()).size(theme::MARK).object_fit(ObjectFit::Contain))
                .child(headline("Willkommen bei BukoSpeak"))
                .child(subline("Wie sollen dich die anderen nennen?", muted))
                .child(
                    v_flex().w_full().mt_6().gap(theme::BEFORE_ACTION).child(field(&form.name)).child(
                        primary("next", "Weiter")
                            .w_full()
                            .disabled(name.is_empty())
                            .on_click(cx.listener(|main, _, window, cx| main.next(window, cx))),
                    ),
                ),
            Step::Place => {
                let complete = [Field::Server, Field::RoomKey].iter().all(|field| !form.value(*field, cx).is_empty());
                v_flex()
                    .w_full()
                    .items_center()
                    .child(Avatar::new(name.clone(), theme::WELCOME_AVATAR))
                    .child(headline(format!("Hallo, {name}")))
                    .child(subline("Wo trefft ihr euch?", muted))
                    .child(
                        v_flex().w_full().mt_6().gap(theme::BETWEEN_FIELDS).child(field(&form.server)).child(field(&form.room_key)).child(
                            div().mt(theme::BEFORE_ACTION - theme::BETWEEN_FIELDS).child(
                                primary("finish", "Beitreten")
                                    .w_full()
                                    .disabled(!complete)
                                    .on_click(cx.listener(|main, _, window, cx| main.finish(window, cx))),
                            ),
                        ),
                    )
            }
        };
        // Forward slides in from the right, back from the left.
        let from = match step {
            Step::Name => -1.,
            Step::Place => 1.,
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .relative()
            .items_center()
            .justify_center()
            .px_6()
            .child(div().w_full().max_w(theme::WELCOME).relative().child(content).with_animation(
                ("step", step as usize),
                Animation::new(theme::APPEAR).with_easing(gpui_kit::ease_out_quint()),
                move |content, delta| content.left(theme::SLIDE * from * (1. - delta)).opacity(delta),
            ))
            .child(self.render_steps(step, cx))
    }

    /// Where in the welcome one is, a dot for each step; a click on one goes there, to the
    /// place once there is a name.
    fn render_steps(&self, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (current, other, hovered) = (theme.foreground, theme.border, theme.muted_foreground);
        let dot = |shown: Step, cx: &mut Context<Self>| {
            let (width, color) = if shown == step { (px(16.), current) } else { (px(6.), other) };
            // The dot is small; the space around it takes the click too.
            div()
                .id(("step-dot", shown as usize))
                .group("step-dot")
                .p_1()
                .cursor_pointer()
                .child(
                    div()
                        .h_1p5()
                        .w(width)
                        .rounded_full()
                        .bg(color)
                        .when(shown != step, |dot| dot.group_hover("step-dot", |dot| dot.bg(hovered))),
                )
                .on_click(cx.listener(move |main, _, window, cx| match shown {
                    Step::Name => main.previous(window, cx),
                    Step::Place => main.next(window, cx),
                }))
        };
        h_flex().absolute().bottom_5().gap_0p5().child(dot(Step::Name, cx)).child(dot(Step::Place, cx))
    }
}

fn headline(text: impl Into<SharedString>) -> impl IntoElement {
    div().mt_5().text_size(theme::TEXT_XL).font_weight(FontWeight::SEMIBOLD).text_center().child(text.into())
}

fn subline(text: &'static str, color: Hsla) -> impl IntoElement {
    div().mt_1p5().text_center().text_color(color).child(text)
}

fn field(input: &Entity<InputState>) -> impl IntoElement {
    Input::new(input).large().context_menu(edit_menu)
}
