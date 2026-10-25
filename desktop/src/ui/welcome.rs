//! The welcome, the first time: the name the others see, then the server and its room key, and
//! in.

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt, Context, Entity, FontWeight, Hsla, IntoElement, ObjectFit, ParentElement, SharedString, Styled, StyledImage,
    div, img, px,
};

use crate::ui::member::Avatar;
use crate::ui::{Back, Field, Form, Main, Step, edit_menu, primary, theme};

impl Main {
    pub(super) fn render_welcome(&self, form: &Form, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let name = form.value(Field::Name, cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let quiet = ButtonCustomVariant::new(cx).foreground(muted).hover(theme.secondary_hover).active(theme.secondary_active);
        let content = match step {
            Step::Name => v_flex()
                .w_full()
                .items_center()
                .child(img(self.mark.clone()).size(theme::MARK).object_fit(ObjectFit::Contain))
                .child(headline("Willkommen bei BukoSpeak"))
                .child(subline("Wie sollen dich die anderen nennen?", muted))
                .child(
                    v_flex().w_full().mt_6().gap_3().child(field(&form.name)).child(
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
                        v_flex()
                            .w_full()
                            .mt_6()
                            .gap_2()
                            .child(field(&form.server))
                            .child(field(&form.room_key))
                            .child(
                                div().mt_1().child(
                                    primary("finish", "Beitreten")
                                        .w_full()
                                        .disabled(!complete)
                                        .on_click(cx.listener(|main, _, window, cx| main.finish(window, cx))),
                                ),
                            )
                            .child(
                                Button::new("back")
                                    .custom(quiet)
                                    .large()
                                    .w_full()
                                    .h(theme::FIELD)
                                    .label("Zurück")
                                    .on_click(cx.listener(|main, _, window, cx| main.back(&Back, window, cx))),
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

    /// Where in the welcome one is: a dot for each step.
    fn render_steps(&self, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dot = |shown: Step| {
            let (width, color) = if shown == step { (px(16.), theme.foreground) } else { (px(6.), theme.border) };
            div().h_1p5().w(width).rounded_full().bg(color)
        };
        h_flex().absolute().bottom_6().gap_1p5().child(dot(Step::Name)).child(dot(Step::Place))
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
