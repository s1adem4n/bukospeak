use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, Window,
    WindowControlArea, div,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, h_flex};
use crate::ui::theme::{self, ActiveTokens};

pub fn sides(window: &Window, cx: &App) -> [Option<AnyElement>; 2] {
    window_buttons(window, cx).map(|side| (!side.is_empty()).then(|| buttons(side, cx).into_any_element()))
}

// No Maximize: the window keeps its size.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Close,
}

impl Caption {
    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "minimize",
            Self::Close => "close",
        }
    }

    fn glyph(self) -> Glyph {
        match self {
            Self::Minimize => Glyph::Minimize,
            Self::Close => Glyph::Close,
        }
    }
}

#[cfg(target_os = "linux")]
fn window_buttons(window: &Window, cx: &App) -> [Vec<Caption>; 2] {
    if !matches!(window.window_decorations(), gpui::Decorations::Client { .. }) {
        return [Vec::new(), Vec::new()];
    }
    let controls = window.window_controls();
    let layout = cx.button_layout().unwrap_or_else(gpui::WindowButtonLayout::linux_default);
    let offered = |buttons: &[Option<gpui::WindowButton>]| {
        buttons
            .iter()
            .flatten()
            .filter_map(|button| match button {
                gpui::WindowButton::Minimize => controls.minimize.then_some(Caption::Minimize),
                gpui::WindowButton::Maximize => None,
                gpui::WindowButton::Close => Some(Caption::Close),
            })
            .collect()
    };
    [offered(&layout.left), offered(&layout.right)]
}

#[cfg(target_os = "windows")]
fn window_buttons(_: &Window, _: &App) -> [Vec<Caption>; 2] {
    [Vec::new(), vec![Caption::Minimize, Caption::Close]]
}

fn buttons(buttons: Vec<Caption>, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    let windows = cfg!(target_os = "windows");
    h_flex().h_full().when(!windows, |side| side.gap(theme::INLINE).px(theme::window::TITLE_INSET)).children(buttons.into_iter().map(
        |button| {
            let close = button == Caption::Close;
            div()
                .id(button.id())
                .flex()
                .items_center()
                .justify_center()
                .text_color(tokens.text_muted)
                .occlude()
                .map(|control| {
                    if windows {
                        let area = match button {
                            Caption::Minimize => WindowControlArea::Min,
                            Caption::Close => WindowControlArea::Close,
                        };
                        control
                            .w(theme::window::CAPTION_BUTTON)
                            .h_full()
                            .window_control_area(area)
                            .when(close, |control| control.hover(|style| style.bg(tokens.danger).text_color(tokens.on_primary)))
                            .when(!close, |control| control.hover(|style| style.bg(tokens.inset).text_color(tokens.text)))
                    } else {
                        control
                            .size(theme::window::BUTTON)
                            .rounded_full()
                            .bg(tokens.inset)
                            .hover(|style| style.bg(tokens.hover).text_color(tokens.text))
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                // Pressed on a button, the title bar doesn't take it for a move.
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                match button {
                                    Caption::Minimize => window.minimize_window(),
                                    Caption::Close => window.remove_window(),
                                }
                            })
                    }
                })
                .child(Icon::new(button.glyph()).size(theme::ICON_SM))
        },
    ))
}
