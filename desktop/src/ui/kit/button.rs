//! The app's button: gpui-base's, which knows clicks, keys and focus, in the app's looks.

use gpui_kit::base::{Button as BaseButton, Selectable, StyledExt};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Background, BoxShadow, ClickEvent, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, Interactivity,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    linear_color_stop, linear_gradient, px,
};

use crate::ui::glyph::Glyph;
use crate::ui::theme::{self, ActiveTokens, Tokens};

/// What a button is for, which its looks tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Lifted off the panel it sits on: most buttons.
    Raised,
    /// The one that moves things on.
    Primary,
    /// What ends something.
    Danger,
    /// Something switched off that wants attention, as a muted microphone.
    Off,
    /// No surface until hovered: an icon in a bar, a row that opens a menu.
    Ghost,
    /// Text alone, which is clicked: a value that opens for editing.
    Bare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Medium,
    Small,
}

#[derive(IntoElement)]
pub struct Button {
    base: BaseButton,
    /// What the caller styled and put in, over the button's own looks and after its icon and
    /// label.
    style: StyleRefinement,
    children: Vec<AnyElement>,
    variant: Variant,
    size: Size,
    square: bool,
    icon: Option<Glyph>,
    label: Option<SharedString>,
    keys: Option<&'static str>,
    tooltip: Option<SharedString>,
    loading: bool,
    disabled: bool,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, variant: Variant) -> Self {
        Self {
            base: BaseButton::new(id),
            style: StyleRefinement::default(),
            children: Vec::new(),
            variant,
            size: Size::Medium,
            square: false,
            icon: None,
            label: None,
            keys: None,
            tooltip: None,
            loading: false,
            disabled: false,
        }
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// As wide as high, for an icon alone.
    pub fn square(mut self) -> Self {
        self.square = true;
        self
    }

    pub fn icon(mut self, icon: Glyph) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The keys that do the same from anywhere, shown after the label.
    pub fn keys(mut self, keys: Option<&'static str>) -> Self {
        self.keys = keys;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Shows that what it started is under way, and takes no other click meanwhile.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Takes the focus by `focus`, so it can be given the focus.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.base = self.base.track_focus(focus);
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.base = self.base.on_click(handler);
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Button {}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.base = self.base.selected(selected);
        self
    }

    fn is_selected(&self) -> bool {
        self.base.is_selected()
    }
}

/// A button opens a menu as gpui-component's does.
impl DropdownMenu for Button {}

/// A fill from its top color to its bottom one.
fn gradient([top, bottom]: [Hsla; 2]) -> Background {
    linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bottom, 1.))
}

/// The light on a raised surface's top edge, `highlight`, and its shadow beneath.
fn lifted(highlight: Hsla, tokens: &Tokens) -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.), px(1.), highlight).inset(), BoxShadow::new(px(0.), px(1.), tokens.shadow).blur_radius(px(2.))]
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let (height, radius, padding, icon) = match self.size {
            Size::Medium => (theme::CONTROL, theme::RADIUS_CONTROL, px(14.), theme::ICON),
            Size::Small => (theme::CONTROL_SM, theme::RADIUS_SMALL, px(10.), theme::ICON_SM),
        };
        let inert = self.disabled || self.loading;
        let text = match self.variant {
            Variant::Raised => tokens.text,
            Variant::Primary => tokens.on_primary,
            Variant::Danger => tokens.primary[0],
            Variant::Off => tokens.danger,
            Variant::Ghost | Variant::Bare => tokens.text_muted,
        };
        let glyph = match (self.loading, self.icon) {
            (true, _) => Some(Spinner::new().color(text).with_size(icon).into_any_element()),
            (false, Some(glyph)) => Some(Icon::from(glyph).size(icon).into_any_element()),
            (false, None) => None,
        };
        let tooltip = self.tooltip;
        let button = self
            .base
            .disabled(inert)
            .h(height)
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(radius)
            .border_1()
            .border_color(tokens.base.opacity(0.))
            .text_size(theme::TEXT)
            .font_weight(FontWeight::MEDIUM)
            .text_color(text)
            .map(|button| match (self.square, self.variant) {
                (true, _) => button.w(height),
                (false, Variant::Bare) => button,
                (false, _) => button.px(padding),
            })
            .focus_visible(|style| style.border_color(tokens.focus))
            .map(|button| style(button, self.variant, tokens))
            .styles(|styles| styles.disabled(|style| style.opacity(if self.loading { 1. } else { theme::DISABLED_OPACITY })))
            .when(!inert, |button| button.cursor_pointer())
            .refine_style(&self.style)
            .children(glyph)
            .children(self.label)
            .children(self.children)
            .children(self.keys.map(|keys| keycap(keys, text)));
        button.when_some(tooltip, |button, tooltip| button.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)))
    }
}

/// The surface of a variant, at rest, hovered and pressed.
fn style(button: BaseButton, variant: Variant, tokens: &Tokens) -> BaseButton {
    match variant {
        Variant::Raised => button
            .bg(gradient(tokens.control))
            .border_color(tokens.edge)
            .shadow(lifted(tokens.highlight, tokens))
            .hover(|style| style.bg(gradient(tokens.control_hover)))
            .active(|style| style.bg(tokens.control[1])),
        Variant::Primary => button
            .bg(gradient(tokens.primary))
            .border_color(tokens.primary_edge)
            .shadow(lifted(tokens.highlight, tokens))
            .hover(|style| style.bg(gradient(tokens.primary_hover)))
            .active(|style| style.bg(tokens.primary[1])),
        Variant::Danger => button
            .bg(gradient(tokens.danger_fill))
            .border_color(tokens.danger_edge)
            .shadow(lifted(tokens.sheen, tokens))
            .hover(|style| style.bg(gradient(tokens.danger_fill_hover)))
            .active(|style| style.bg(tokens.danger_fill[1])),
        Variant::Off => button
            .bg(tokens.danger.opacity(0.14))
            .border_color(tokens.danger.opacity(0.35))
            .hover(|style| style.bg(tokens.danger.opacity(0.2)))
            .active(|style| style.bg(tokens.danger.opacity(0.26))),
        Variant::Ghost => button
            .hover(|style| style.bg(tokens.inset).text_color(tokens.text))
            .active(|style| style.bg(tokens.hover))
            .styles(|styles| styles.selected(|style| style.bg(tokens.inset).text_color(tokens.text))),
        Variant::Bare => button.hover(|style| style.text_color(tokens.text)),
    }
}

/// Keys in a small chip, in the color of the text they sit beside.
fn keycap(keys: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .px_1p5()
        .rounded(theme::RADIUS_TINY)
        .border_1()
        .border_color(color.opacity(0.2))
        .font_family(theme::MONO)
        .text_size(theme::TEXT_XS)
        .line_height(theme::KEYCAP)
        .text_color(color.opacity(0.65))
        .child(keys)
}

/// How far a square button's edge lies outside its icon: pulled out by it, the icon lines up
/// with what is above or beside the button.
pub fn icon_inset(size: Size) -> Pixels {
    match size {
        Size::Medium => (theme::CONTROL - theme::ICON) / 2.,
        Size::Small => (theme::CONTROL_SM - theme::ICON_SM) / 2.,
    }
}
