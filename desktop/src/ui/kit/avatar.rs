//! A member's avatar: their initial on the color their name gives, and in the channel the ring
//! that lights up while they speak.

use gpui_kit::{
    Animation, AnimationExt, App, BoxShadow, ElementId, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled,
    Window, div, px,
};

use crate::ui::theme::{self, ActiveTokens};

#[derive(IntoElement)]
pub struct Avatar {
    name: SharedString,
    size: Pixels,
    /// The member's id, which keeps the ring's fade across renders, and whether it is lit.
    ring: Option<(u64, bool)>,
}

impl Avatar {
    pub fn new(name: impl Into<SharedString>, size: Pixels) -> Self {
        Self { name: name.into(), size, ring: None }
    }

    /// Draws the speaking ring, lit while `speaks`, fading in and out.
    pub fn ring(mut self, member: u64, speaks: bool) -> Self {
        self.ring = Some((member, speaks));
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let (background, initial_color) = theme::avatar(&self.name, tokens.dark);
        let initial: String = self.name.chars().next().into_iter().flat_map(char::to_uppercase).collect();
        let speaking = tokens.speaking;
        let ring = self.ring.map(|(member, speaks)| {
            // A new id for each change starts the fade anew, in or out.
            let id = ElementId::from(("ring", member << 1 | u64::from(speaks)));
            div().absolute().inset(-theme::RING * 2.).rounded_full().border(theme::RING).border_color(speaking).with_animation(
                id,
                Animation::new(theme::SPEAKING_FADE).with_easing(gpui_kit::ease_out_quint()),
                move |ring, delta| ring.opacity(if speaks { delta } else { 1. - delta }),
            )
        });
        div()
            .relative()
            .flex_none()
            .size(self.size)
            .rounded_full()
            .bg(background)
            // An edge of light, so a dark avatar doesn't melt into the panel.
            .shadow(vec![BoxShadow::new(px(0.), px(0.), tokens.hairline).spread_radius(px(1.)).inset()])
            .text_color(initial_color)
            // One line as high as the circle: Geist puts a capital's middle on the middle of its
            // line, so the letter is centered to the pixel, which centering a box around the
            // letter's own fractional height is not.
            .text_center()
            .line_height(self.size)
            .text_size((self.size * 0.44).round())
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial)
            .children(ring)
    }
}
