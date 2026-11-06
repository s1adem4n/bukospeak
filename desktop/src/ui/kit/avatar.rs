//! A member's avatar: their initial on the color their name gives, and a badge on its edge for what holds them back: muted, reconnecting.

use gpui::{App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};

use crate::ui::glyph::Glyph;
use crate::ui::kit::Icon;
use crate::ui::theme::{self, ActiveTokens, AvatarSize, Typeset};

#[derive(IntoElement)]
pub struct Avatar {
    name: SharedString,
    size: AvatarSize,
    /// The badge's glyph and color, and the color of what the avatar is on, which the badge is
    /// cut out of.
    badge: Option<(Glyph, Hsla, Hsla)>,
}

impl Avatar {
    pub fn new(name: impl Into<SharedString>, size: AvatarSize) -> Self {
        Self { name: name.into(), size, badge: None }
    }

    /// Puts `glyph` in `color` on the avatar's edge, cut out of `on`, what the avatar is on.
    pub fn badge(mut self, glyph: Glyph, color: Hsla, on: Hsla) -> Self {
        self.badge = Some((glyph, color, on));
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let AvatarSize { size, initial: kind, badge: badge_size } = self.size;
        let (background, initial_color) = theme::avatar(&self.name, tokens.dark);
        let initial: String = self.name.chars().next().into_iter().flat_map(char::to_uppercase).collect();
        let badge = self.badge.map(|(glyph, color, on)| {
            div()
                .absolute()
                .right(-theme::BADGE_EDGE)
                .bottom(-theme::BADGE_EDGE)
                .size(badge_size)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(on.blend(color.opacity(0.16)))
                .border(theme::BADGE_EDGE)
                .border_color(on)
                .child(Icon::new(glyph).size(theme::ICON_SM).color(color))
        });
        div()
            .relative()
            .flex_none()
            .size(size)
            .rounded_full()
            .bg(background)
            // An edge of light, so a dark avatar doesn't melt into the panel.
            .shadow(theme::depth::inner_edge(tokens.hairline))
            .text_color(initial_color)
            .typeset(kind)
            // One line as high as the circle: Geist puts a capital's middle on the middle of its
            // line, so the letter is centered to the pixel, which centering a box around the
            // letter's own fractional height is not.
            .text_center()
            .line_height(size)
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial)
            .children(badge)
    }
}
