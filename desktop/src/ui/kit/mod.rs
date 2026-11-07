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
    AnyElement, App, ClickEvent, Div, Entity, FontWeight, Hsla, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Pixels, Styled,
    StyledImage, Window, div, img, relative,
};
pub use icon::{Icon, Spinner};
pub use level::InputLevel;
use menu::{Menu, Place, Popover};
pub use slider::{Slider, SliderEvent};
pub use switch::Switch;
pub use text_field::{TextField, TextFieldEvent};
pub use tooltip::Tooltip;

use crate::ui::glyph::Glyph;
use crate::ui::theme::{self, ActiveTokens, Typeset};

pub fn init(cx: &mut App) {
    field::init(cx);
    menu::init(cx);
    slider::init(cx);
    text_field::init(cx);
}

pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}

pub fn panel(cx: &App) -> Div {
    raise(div(), cx)
}

fn raise(surface: Div, cx: &App) -> Div {
    let tokens = cx.tokens();
    surface.bg(tokens.raised).border_1().border_color(tokens.hairline).rounded(theme::RADIUS_PANEL).shadow(theme::depth::panel(tokens))
}

pub fn led(mark: impl IntoElement) -> Div {
    h_flex().gap(theme::INLINE).child(h_flex().flex_none().size(theme::ICON_SM).justify_center().child(mark))
}

pub fn callout(mark: impl IntoElement, cx: &App) -> Div {
    raise(led(mark), cx).flex_none().pl(panel_inset())
}

pub fn list(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let hairline = cx.tokens().hairline;
    let rows =
        rows.into_iter().enumerate().map(|(index, row)| div().when(index > 0, |line| line.border_t_1().border_color(hairline)).child(row));
    panel(cx).px(theme::PANEL_PAD).flex().flex_col().children(rows)
}

pub fn list_reach() -> Pixels {
    theme::PANEL_PAD + theme::HAIRLINE
}

pub fn panel_inset() -> Pixels {
    theme::PANEL_INSET - theme::HAIRLINE
}

pub fn row(label: &'static str, cx: &App) -> Div {
    h_flex()
        .h(theme::ROW)
        .px(panel_inset() - theme::PANEL_PAD)
        .gap(theme::GAP)
        .justify_between()
        .text_color(cx.tokens().text)
        .child(div().flex_none().child(label))
}

pub fn menu_row(
    id: &'static str,
    label: &'static str,
    value: impl IntoElement,
    menu: Option<&Entity<Menu>>,
    open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let tokens = cx.tokens();
    // Its surface lies as far inside the row as the row inside the panel's sides, so its corners
    // nest in the panel's.
    let row = Button::new(id, Variant::Ghost)
        .w_full()
        .my(theme::PANEL_PAD)
        .h(theme::ROW - theme::PANEL_PAD * 2.)
        // The button's own edge lies inside the panel's.
        .px(panel_inset() - theme::PANEL_PAD - theme::HAIRLINE)
        .gap(theme::GAP)
        .font_weight(FontWeight::NORMAL)
        .child(div().flex_none().text_color(tokens.text).child(label))
        // A long value, as a device's name, gives way to the label and is cut short.
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .justify_end()
                .gap(theme::TIGHT)
                .text_color(tokens.text_muted)
                .child(div().min_w_0().truncate().child(value))
                .child(Icon::new(Glyph::Choose).size(theme::ICON_SM).color(tokens.text_faint)),
        )
        .on_click(open);
    div().relative().child(row).children(menu.map(|menu| Popover::new(menu, Place::OverValue))).into_any_element()
}

pub fn group(title: &'static str, rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let title = div()
        .px(theme::PANEL_INSET)
        .typeset(theme::TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.tokens().text_faint)
        .child(title);
    v_flex().gap(theme::CAPTION).child(title).child(list(rows, cx))
}

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

static MARK: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../../assets/mark.png").to_vec())));

pub fn mark(height: Pixels) -> impl IntoElement {
    img(MARK.clone()).flex_none().h(height).object_fit(ObjectFit::Contain)
}
