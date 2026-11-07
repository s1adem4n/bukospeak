use std::sync::Arc;

use gpui::{
    App, FontWeight, IntoElement, ObjectFit, ParentElement, RenderImage, RenderOnce, SharedString, Styled, StyledImage, Window, div, img,
};

use crate::ui::theme::{self, ActiveTokens, Typeset};

/// A program's icon, or where it has none, its initial on a tile of its own color.
#[derive(Clone, IntoElement)]
pub struct AppIcon {
    name: SharedString,
    image: Option<Arc<RenderImage>>,
    size: theme::app_icon::Size,
}

impl AppIcon {
    pub fn new(name: impl Into<SharedString>, image: Option<Arc<RenderImage>>, size: theme::app_icon::Size) -> Self {
        Self { name: name.into(), image, size }
    }
}

impl RenderOnce for AppIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme::app_icon::Size { size, radius, initial } = self.size;
        if let Some(image) = self.image {
            return img(image).flex_none().size(size).object_fit(ObjectFit::Contain).into_any_element();
        }
        let tokens = cx.tokens();
        let (background, color) = theme::avatar(&self.name, tokens.dark);
        let letter: String = self.name.chars().next().into_iter().flat_map(char::to_uppercase).collect();
        div()
            .flex_none()
            .size(size)
            .rounded(radius)
            .bg(background)
            .text_color(color)
            .shadow(theme::depth::inner_edge(tokens.hairline))
            .typeset(initial)
            // As on an avatar: a line as high as the tile centers the capital to the pixel.
            .text_center()
            .line_height(size)
            .font_weight(FontWeight::SEMIBOLD)
            .child(letter)
            .into_any_element()
    }
}
