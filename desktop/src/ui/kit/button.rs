//! The app's button: a surface in the looks of what it is for, which is clicked, or pressed with
//! Enter or Space once Tab gave it the focus.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    Refineable, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, Spinner, Tooltip};
use crate::ui::marks::Mark;
use crate::ui::theme::{self, ActiveTokens, Tokens, Type, Typeset};

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
    /// In bars and rows.
    Small,
    Medium,
    /// What a screen is for: the dock's, the welcome's.
    Large,
}

/// A size's height, corners, inset of its content, icon and type: of buttons, and of the fields
/// beside them.
pub(super) struct Metrics {
    pub height: Pixels,
    pub radius: Pixels,
    pub inset: Pixels,
    pub icon: Pixels,
    pub text: Type,
}

impl Size {
    pub(super) fn metrics(self) -> Metrics {
        match self {
            Self::Small => Metrics {
                height: theme::CONTROL_SM,
                radius: theme::RADIUS_CONTROL_SM,
                inset: theme::CONTROL_SM_INSET,
                icon: theme::ICON_SM,
                text: theme::TEXT,
            },
            Self::Medium => Metrics {
                height: theme::CONTROL,
                radius: theme::RADIUS_CONTROL,
                inset: theme::CONTROL_INSET,
                icon: theme::ICON,
                text: theme::TEXT,
            },
            Self::Large => Metrics {
                height: theme::CONTROL_LG,
                radius: theme::RADIUS_CONTROL_LG,
                inset: theme::CONTROL_LG_INSET,
                icon: theme::ICON_LG,
                text: theme::TEXT_MD,
            },
        }
    }
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
    /// The focus it takes, if not one of its own.
    focus: Option<FocusHandle>,
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
            focus: None,
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

    /// Takes the focus by `focus`, so it can be given the focus.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
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

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let Metrics { height, radius, inset, icon, text: kind } = self.size.metrics();
        let inert = self.disabled || self.loading;
        // A disabled fill would still call for the click it doesn't take: it gives the fill up
        // and lies flat, without an edge, which would make it a field; its text faint.
        let given_up = self.disabled && matches!(self.variant, Variant::Primary | Variant::Danger);
        let text = match self.variant {
            _ if given_up => tokens.text_faint,
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
        let mark = self.id.to_string();
        let mut button = div()
            .id(self.id)
            .mark(mark)
            .role(Role::Button)
            .when_some(name, |button, name| button.aria_label(name))
            .h(height)
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(radius)
            // A bare button is its content alone, lined up as text is; the others keep an edge
            // that focus colors.
            .when(self.variant != Variant::Bare, |button| button.border_1().border_color(tokens.base.opacity(0.)))
            .typeset(kind)
            .font_weight(FontWeight::MEDIUM)
            .text_color(text)
            .map(|button| match (self.square, self.variant) {
                (true, _) => button.w(height),
                (false, Variant::Bare) => button,
                // Inside its edge, which takes room too: the content lies `inset` off the outside.
                (false, _) => button.px(inset - theme::HAIRLINE),
            })
            .map(|button| match given_up {
                true => button.bg(tokens.hover),
                false => surface(button, self.variant, self.size, tokens),
            });
        if inert {
            // A loading button shows its spinner in full; a disabled one without a fill fades.
            button = button.when(self.disabled && !given_up, |button| button.opacity(theme::DISABLED_OPACITY));
        } else {
            button = button
                .cursor_pointer()
                .map(|button| hovered(button, self.variant, tokens))
                .map(|button| {
                    let button = match &self.focus {
                        Some(focus) => button.track_focus(focus),
                        None => button.tab_index(0),
                    };
                    // What has no surface shows the focus as it shows the pointer; the rest by its edge.
                    match self.variant {
                        Variant::Ghost => button.focus_visible(|style| style.bg(tokens.inset).text_color(tokens.text)),
                        Variant::Bare => button.focus_visible(|style| style.text_color(tokens.text)),
                        _ => button.focus_visible(|style| style.border_color(tokens.focus)),
                    }
                })
                .when_some(self.on_click, |button, handler| button.on_click(handler))
                .when_some(self.tooltip, |button, tooltip| button.tooltip(Tooltip::text(tooltip)));
        }
        button.style().refine(&self.style);
        button.children(glyph).children(self.label).children(self.children).children(self.keys.map(|keys| keycap(keys, text)))
    }
}

/// The surface of a variant at rest.
fn surface(button: Stateful<gpui::Div>, variant: Variant, size: Size, tokens: &Tokens) -> Stateful<gpui::Div> {
    // A large button stands further off what it is on.
    let lifted = |light| if size == Size::Large { theme::depth::control_lg(light, tokens) } else { theme::depth::control(light, tokens) };
    match variant {
        Variant::Raised => button.bg(theme::gradient(tokens.control)).border_color(tokens.edge).shadow(lifted(tokens.highlight)),
        Variant::Primary => button.bg(theme::gradient(tokens.primary)).border_color(tokens.primary_edge).shadow(lifted(tokens.highlight)),
        Variant::Danger => button.bg(theme::gradient(tokens.danger_fill)).border_color(tokens.danger_edge).shadow(lifted(tokens.sheen)),
        Variant::Off => button.bg(tokens.danger.opacity(0.14)).border_color(tokens.danger.opacity(0.35)),
        Variant::Ghost | Variant::Bare => button,
    }
}

/// How a variant answers the pointer: hovered, and pressed.
fn hovered(button: Stateful<gpui::Div>, variant: Variant, tokens: &Tokens) -> Stateful<gpui::Div> {
    match variant {
        Variant::Raised => {
            button.hover(|style| style.bg(theme::gradient(tokens.control_hover))).active(|style| style.bg(tokens.control[1]))
        }
        Variant::Primary => {
            button.hover(|style| style.bg(theme::gradient(tokens.primary_hover))).active(|style| style.bg(tokens.primary[1]))
        }
        Variant::Danger => {
            button.hover(|style| style.bg(theme::gradient(tokens.danger_fill_hover))).active(|style| style.bg(tokens.danger_fill[1]))
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
        .rounded(theme::RADIUS_XS)
        .border_1()
        .border_color(color.opacity(0.2))
        .font_family(theme::MONO)
        .typeset(theme::TEXT_XS)
        .line_height(theme::KEYCAP)
        .text_color(color.opacity(0.65))
        .child(keys)
}

/// How far a square button's edge lies outside its icon: pulled out by it, the icon lines up
/// with what is above or beside the button.
pub fn icon_inset(size: Size) -> Pixels {
    let Metrics { height, icon, .. } = size.metrics();
    (height - icon) / 2.
}
