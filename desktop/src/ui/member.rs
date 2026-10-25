//! A member as the app shows them: their initial on their color, and in the channel a row with
//! the ring that lights up while they speak.

use gpui_kit::component::{ActiveTheme, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Animation, AnimationExt, App, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled, Window, div, relative,
};

use crate::ui::glyph::Glyph;
use crate::ui::theme;

/// A member's initial on a circle of the color their name gives.
#[derive(IntoElement)]
pub struct Avatar {
    name: SharedString,
    size: Pixels,
}

impl Avatar {
    pub fn new(name: impl Into<SharedString>, size: Pixels) -> Self {
        Self { name: name.into(), size }
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (background, initial_color) = theme::avatar(&self.name, cx.theme().is_dark());
        let initial: String = self.name.chars().next().into_iter().flat_map(char::to_uppercase).collect();
        div()
            .size(self.size)
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(background)
            .text_color(initial_color)
            .text_size(self.size * 0.44)
            // As high as the letter, so it sits in the middle and not on the line's lower part.
            .line_height(relative(1.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial)
    }
}

/// A member in the channel: the avatar in its speaking ring, the name, and on the right a mark
/// while muted or reconnecting, when they are faded.
#[derive(IntoElement)]
pub struct Row {
    id: u16,
    name: SharedString,
    speaks: bool,
    muted: bool,
    reconnecting: bool,
    yours: bool,
}

impl Row {
    pub fn new(id: u16, name: impl Into<SharedString>, speaks: bool, muted: bool, reconnecting: bool) -> Self {
        Self { id, name: name.into(), speaks, muted, reconnecting, yours: false }
    }

    /// Marks the row as the user's own.
    pub fn yours(mut self, yours: bool) -> Self {
        self.yours = yours;
        self
    }
}

impl RenderOnce for Row {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let speaking = theme::speaking(theme.is_dark());
        // A new id for each change starts the fade anew, in or out.
        let ring = div().absolute().inset_0().rounded_full().border(theme::RING).border_color(speaking).with_animation(
            ("ring", u64::from(self.id) << 1 | u64::from(self.speaks)),
            Animation::new(theme::SPEAKING_FADE).with_easing(gpui_kit::ease_out_quint()),
            move |ring, delta| ring.opacity(if self.speaks { delta } else { 1. - delta }),
        );
        let mark = match (self.reconnecting, self.muted) {
            (true, _) => Some((Glyph::Reconnecting, theme.muted_foreground)),
            (false, true) => Some((Glyph::MicOff, theme.danger)),
            (false, false) => None,
        };
        div()
            .h(theme::ROW)
            .px_2()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded(theme.radius)
            .when(self.reconnecting, |row| row.opacity(theme::RECONNECTING_OPACITY))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .p(theme::RING * 2.)
                    .m(-theme::RING * 2.)
                    .child(Avatar::new(self.name.clone(), theme::ROW_AVATAR))
                    .child(ring),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(self.name))
                    .when(self.yours, |name| {
                        name.child(div().flex_none().text_size(theme::TEXT_SM).text_color(theme.muted_foreground).child("du"))
                    }),
            )
            .children(mark.map(|(mark, color)| Icon::from(mark).size(theme::MARK_ICON).text_color(color)))
    }
}
