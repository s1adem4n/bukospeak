//! The app's button: a surface in the looks of what it is for, which is clicked, or pressed with
//! Enter or Space once Tab gave it the focus.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Background, BoxShadow, ClickEvent, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, Refineable, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled,
    Window, div, linear_color_stop, linear_gradient, px,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, Spinner, Tooltip};
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

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    /// What the caller styled, over the button's own looks.
    style: StyleRefinement,
    /// What the caller put in, after the icon and the label.
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
    selected: bool,
    /// The focus it takes, if not one of its own.
    focus: Option<FocusHandle>,
    /// Whether Tab and a click give it the focus.
    focusable: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, variant: Variant) -> Self {
        Self {
            id: id.into(),
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
            selected: false,
            focus: None,
            focusable: true,
            on_click: None,
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

    /// Shows what it opened as open: a ghost button keeps its hovered surface.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Takes the focus by `focus`, so it can be given the focus.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    /// Leaves the focus where it is when clicked, and is passed over by Tab: a control inside a
    /// field, which keeps the focus.
    pub fn unfocusable(mut self) -> Self {
        self.focusable = false;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

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
            (true, _) => Some(Spinner::new(icon).color(text).into_any_element()),
            (false, Some(glyph)) => Some(Icon::new(glyph).size(icon).into_any_element()),
            (false, None) => None,
        };
        let name = self.label.clone().or_else(|| self.tooltip.clone());
        let mut button = div()
            .id(self.id)
            .role(Role::Button)
            .when_some(name, |button, name| button.aria_label(name))
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
            .map(|button| surface(button, self.variant, self.selected, tokens));
        if inert {
            // A loading button shows its spinner in full; a disabled one fades.
            button = button.when(self.disabled, |button| button.opacity(theme::DISABLED_OPACITY));
        } else {
            button = button
                .cursor_pointer()
                .map(|button| hovered(button, self.variant, tokens))
                .when(self.focusable, |button| {
                    let button = match &self.focus {
                        Some(focus) => button.track_focus(focus),
                        None => button.tab_index(0),
                    };
                    button.focus_visible(|style| style.border_color(tokens.focus))
                })
                .when_some(self.on_click, |button, handler| button.on_click(handler))
                .when_some(self.tooltip, |button, tooltip| button.tooltip(Tooltip::text(tooltip)));
        }
        button.style().refine(&self.style);
        button.children(glyph).children(self.label).children(self.children).children(self.keys.map(|keys| keycap(keys, text)))
    }
}

/// The surface of a variant at rest, or selected.
fn surface(button: Stateful<gpui::Div>, variant: Variant, selected: bool, tokens: &Tokens) -> Stateful<gpui::Div> {
    match variant {
        Variant::Raised => button.bg(gradient(tokens.control)).border_color(tokens.edge).shadow(lifted(tokens.highlight, tokens)),
        Variant::Primary => button.bg(gradient(tokens.primary)).border_color(tokens.primary_edge).shadow(lifted(tokens.highlight, tokens)),
        Variant::Danger => button.bg(gradient(tokens.danger_fill)).border_color(tokens.danger_edge).shadow(lifted(tokens.sheen, tokens)),
        Variant::Off => button.bg(tokens.danger.opacity(0.14)).border_color(tokens.danger.opacity(0.35)),
        Variant::Ghost => button.when(selected, |button| button.bg(tokens.inset).text_color(tokens.text)),
        Variant::Bare => button,
    }
}

/// How a variant answers the pointer: hovered, and pressed.
fn hovered(button: Stateful<gpui::Div>, variant: Variant, tokens: &Tokens) -> Stateful<gpui::Div> {
    match variant {
        Variant::Raised => button.hover(|style| style.bg(gradient(tokens.control_hover))).active(|style| style.bg(tokens.control[1])),
        Variant::Primary => button.hover(|style| style.bg(gradient(tokens.primary_hover))).active(|style| style.bg(tokens.primary[1])),
        Variant::Danger => {
            button.hover(|style| style.bg(gradient(tokens.danger_fill_hover))).active(|style| style.bg(tokens.danger_fill[1]))
        }
        Variant::Off => button.hover(|style| style.bg(tokens.danger.opacity(0.2))).active(|style| style.bg(tokens.danger.opacity(0.26))),
        Variant::Ghost => button.hover(|style| style.bg(tokens.inset).text_color(tokens.text)).active(|style| style.bg(tokens.hover)),
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
