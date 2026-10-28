//! What the screens are built of: surfaces, lists, buttons, fields and the small marks between.
//! The looks live here and in the theme; a screen only puts the pieces together.

mod avatar;
mod button;
mod field;
mod level;

pub use avatar::Avatar;
pub use button::{Button, Size, Variant, icon_inset};
pub use field::{InlineField, InlineFieldEvent, Look, field, init};
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::FluentBuilder;
pub use level::InputLevel;
use std::sync::{Arc, LazyLock};

use gpui_kit::{
    AnyElement, App, BoxShadow, Div, FontWeight, Hsla, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Pixels, SharedString,
    Styled, StyledImage, div, img, px, relative,
};

use crate::ui::glyph::Glyph;
use crate::ui::theme::{self, ActiveTokens};

/// A panel: raised off the window, inside a hairline.
pub fn panel(cx: &App) -> Div {
    let tokens = cx.tokens();
    div().bg(tokens.raised).border_1().border_color(tokens.hairline).rounded(theme::RADIUS_PANEL)
}

/// A panel of rows one under the other, a hairline between each two.
pub fn list(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let hairline = cx.tokens().hairline;
    let rows =
        rows.into_iter().enumerate().map(|(index, row)| div().when(index > 0, |line| line.border_t_1().border_color(hairline)).child(row));
    panel(cx).p(theme::PANEL_PAD).flex().flex_col().children(rows)
}

/// A row of a list: its label at the panel's inset, and at its end what it is set to.
pub fn row(label: &'static str, cx: &App) -> Div {
    h_flex()
        .h(theme::ROW)
        .px(theme::PANEL_INSET - theme::PANEL_PAD)
        .gap_3()
        .justify_between()
        .text_color(cx.tokens().text)
        .child(div().flex_none().child(label))
}

/// The title of the panel below it.
pub fn section(title: &'static str, cx: &App) -> impl IntoElement {
    div()
        .pt_2()
        .px(theme::PANEL_INSET)
        .text_size(theme::TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.tokens().text_faint)
        .child(title)
}

/// A small count or code in a chip.
pub fn chip(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    div()
        .flex_none()
        .px_1p5()
        .rounded(theme::RADIUS_TINY)
        .bg(tokens.inset)
        .border_1()
        .border_color(tokens.hairline)
        .font_family(theme::MONO)
        .text_size(theme::TEXT_XS)
        .line_height(theme::CHIP)
        .text_color(tokens.text_muted)
        .child(text.into())
}

/// An icon on a tile set into the panel, heading something empty.
pub fn tile(glyph: Glyph, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    div()
        .flex_none()
        .size(theme::CONTROL)
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .bg(tokens.inset)
        .border_1()
        .border_color(tokens.hairline)
        .text_color(tokens.text_muted)
        .child(Icon::from(glyph).size(theme::ICON_LG))
}

/// A status's dot, in a halo of its color.
pub fn dot(color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(7.))
        .rounded_full()
        .bg(color)
        .shadow(vec![BoxShadow::new(px(0.), px(0.), color.opacity(0.16)).spread_radius(px(3.))])
}

/// How far something is along, `done` of it from 0 to 1, in a bar set into what it is on.
pub fn progress(done: f32, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    div()
        .w_full()
        .h(theme::PROGRESS)
        .rounded_full()
        .bg(tokens.inset)
        .border_1()
        .border_color(tokens.hairline)
        .child(div().h_full().w(relative(done)).rounded_full().bg(tokens.primary[0]))
}

/// The app's mark, the microphone of its icon cut out, as the iOS app shows it.
static MARK: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../../assets/mark.png").to_vec())));

/// The app's mark, `height` high.
pub fn mark(height: Pixels) -> impl IntoElement {
    img(MARK.clone()).flex_none().h(height).object_fit(ObjectFit::Contain)
}

/// The app's mark and name, as the title bar shows them.
pub fn wordmark() -> impl IntoElement {
    h_flex().gap_1p5().font_weight(FontWeight::SEMIBOLD).child(mark(theme::MARK_SM)).child("BukoSpeak")
}
