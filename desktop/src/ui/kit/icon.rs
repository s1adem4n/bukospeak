use gpui::{Animation, AnimationExt, App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Svg, Transformation, Window, percentage, svg};

use crate::ui::glyph::Glyph;
use crate::ui::theme;

#[derive(IntoElement)]
pub struct Icon {
    glyph: Glyph,
    size: Pixels,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(glyph: Glyph) -> Self {
        Self { glyph, size: theme::ICON, color: None }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    // An SVG takes no color from the text around it.
    fn svg(self, window: &Window) -> Svg {
        let color = self.color.unwrap_or_else(|| window.text_style().color);
        svg().path(self.glyph.path()).flex_none().size(self.size).text_color(color)
    }
}

impl From<Glyph> for Icon {
    fn from(glyph: Glyph) -> Self {
        Self::new(glyph)
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        self.svg(window)
    }
}

#[derive(IntoElement)]
pub struct Spinner {
    icon: Icon,
}

impl Spinner {
    pub fn new(size: Pixels) -> Self {
        Self { icon: Icon::new(Glyph::Loading).size(size) }
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.icon = self.icon.color(color);
        self
    }
}

impl RenderOnce for Spinner {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        self.icon.svg(window).with_animation("spinner", Animation::new(theme::SPIN).repeat(), |svg, turned| {
            svg.with_transformation(Transformation::rotate(percentage(turned)))
        })
    }
}
