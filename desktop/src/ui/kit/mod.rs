//! What the screens are built of: surfaces, lists, buttons, fields and the small marks between.
//! The looks live here and in the theme; a screen only puts the pieces together.

mod avatar;
mod button;
mod field;
mod icon;
mod level;
pub mod menu;
mod slider;
mod switch;
mod text_field;
mod tooltip;

use std::sync::{Arc, LazyLock};

pub use avatar::Avatar;
pub use button::{Button, Size, Variant, icon_inset};
pub use field::{Field, InlineField, InlineFieldEvent};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Div, FontWeight, Hsla, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Pixels, Styled, StyledImage, div,
    img, relative,
};
pub use icon::{Icon, Spinner};
pub use level::InputLevel;
pub use slider::{Slider, SliderEvent};
pub use switch::Switch;
pub use text_field::{TextField, TextFieldEvent};
pub use tooltip::Tooltip;

use crate::ui::glyph::Glyph;
use crate::ui::theme::{self, ActiveTokens, Typeset};

/// Binds the keys of the pieces that take them.
pub fn init(cx: &mut App) {
    field::init(cx);
    menu::init(cx);
    slider::init(cx);
    text_field::init(cx);
}

/// A row: its children side by side, centered across it.
pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

/// A column: its children one under the other.
pub fn v_flex() -> Div {
    div().flex().flex_col()
}

/// A panel: raised off the window, inside a hairline.
pub fn panel(cx: &App) -> Div {
    raise(div(), cx)
}

/// Raises `surface` off the window as a panel.
fn raise(surface: Div, cx: &App) -> Div {
    let tokens = cx.tokens();
    surface.bg(tokens.raised).border_1().border_color(tokens.hairline).rounded(theme::RADIUS_PANEL).shadow(theme::depth::panel(tokens))
}

/// A line led by `mark` in an icon's place, so the text after it starts where every such
/// line's does, whatever the mark.
pub fn led(mark: impl IntoElement) -> Div {
    h_flex().gap_2p5().child(h_flex().flex_none().size(theme::ICON_SM).justify_center().child(mark))
}

/// A panel of one line, led by `mark` at the panel's inset.
pub fn callout(mark: impl IntoElement, cx: &App) -> Div {
    raise(led(mark), cx).flex_none().pl(panel_inset())
}

/// A panel of rows one under the other, a hairline between each two. Its padding is at its
/// sides only: each row is as high as the others from edge or hairline to hairline or edge, and
/// what highlights a row lies inside the row.
pub fn list(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let hairline = cx.tokens().hairline;
    let rows =
        rows.into_iter().enumerate().map(|(index, row)| div().when(index > 0, |line| line.border_t_1().border_color(hairline)).child(row));
    panel(cx).px(theme::PANEL_PAD).flex().flex_col().children(rows)
}

/// How far a panel's content lies inside its edge, which takes room too: `PANEL_INSET` off its
/// outside, where a section's title lines up.
pub fn panel_inset() -> Pixels {
    theme::PANEL_INSET - theme::HAIRLINE
}

/// A row of a list, its content from the panel's inset to the same distance off its end.
pub fn row_frame() -> Div {
    h_flex().h(theme::ROW).px(panel_inset() - theme::PANEL_PAD).gap_3()
}

/// A row of a list: its label at the panel's inset, and at its end what it is set to.
pub fn row(label: &'static str, cx: &App) -> Div {
    row_frame().justify_between().text_color(cx.tokens().text).child(div().flex_none().child(label))
}

/// A row of a list that opens a menu: what it is at the panel's inset, at its end what it is
/// set to, if that isn't what it is, and the menu's mark after, lit while the menu is `open`.
pub fn menu_row(id: &'static str, lead: impl IntoElement, value: Option<AnyElement>, open: bool, cx: &App) -> Button {
    let tokens = cx.tokens();
    // Its surface lies as far inside the row as the row inside the panel's sides, so its corners
    // nest in the panel's.
    Button::new(id, Variant::Ghost)
        .w_full()
        .my(theme::PANEL_PAD)
        .h(theme::ROW - theme::PANEL_PAD * 2.)
        // The button's own edge lies inside the panel's.
        .px(panel_inset() - theme::PANEL_PAD - theme::HAIRLINE)
        .gap_3()
        .font_weight(FontWeight::NORMAL)
        .child(div().flex_1().min_w_0().text_color(tokens.text).child(lead))
        .child(h_flex().flex_none().gap_1p5().children(value).child(Icon::new(Glyph::Choose).size(theme::ICON_SM).color(if open {
            tokens.text
        } else {
            tokens.text_faint
        })))
}

/// A list under its title, which lines up with the list's rows.
pub fn group(title: &'static str, rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let title = div()
        .px(theme::PANEL_INSET)
        .typeset(theme::TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.tokens().text_faint)
        .child(title);
    v_flex().gap_2().child(title).child(list(rows, cx))
}

/// A status's dot, in a halo of its color.
pub fn dot(color: Hsla) -> impl IntoElement {
    // A circle of its own: a shadow spread around a circle comes out a rounded square.
    div()
        .flex_none()
        .size(theme::DOT + theme::DOT_HALO * 2.)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color.opacity(0.16))
        .child(div().size(theme::DOT).rounded_full().bg(color))
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
