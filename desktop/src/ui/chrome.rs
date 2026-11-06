//! The window's chrome, which the app draws itself on every system: the title bar, which holds
//! the screen's controls, moves the window and, where the system leaves it to the app, holds the
//! window's buttons; and on Linux, where the compositor leaves the frame to the app, the
//! frame's rounded edge, its shadow, and the margin in it that resizes the window. After Zed's
//! `platform_title_bar` and `workspace::client_side_decorations`.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Div, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    RenderOnce, ResizeEdge, Size, Stateful, StatefulInteractiveElement, Styled, Tiling, TitlebarOptions, Window, WindowButton,
    WindowControlArea, WindowDecorations, WindowOptions, div, point, transparent_black,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, h_flex};
use crate::ui::theme::{self, ActiveTokens};

/// A window of the app, `size` large in the middle of the screen, with the chrome drawn by the
/// app.
pub fn options(size: Size<Pixels>, cx: &App) -> WindowOptions {
    let light = (theme::TITLE_BAR - theme::TRAFFIC_LIGHT) / 2.;
    WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some("BukoSpeak".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(light, light)),
        }),
        // The title bar moves the window itself, so a click on it isn't held back on macOS
        // while the system waits to tell a drag from a double click.
        app_owns_titlebar_drag: true,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("bukospeak".into()),
        ..WindowOptions::default()
    }
}

/// The frame around a window's content: on Linux, while the app draws it, a rounded edge with a
/// shadow and the margin around it that takes the drags resizing the window, on every side not
/// tiled against the screen's edge or another window. Elsewhere the content alone.
pub fn frame(content: impl IntoElement, window: &mut Window, cx: &App) -> Stateful<Div> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        window.set_client_inset(Pixels::ZERO);
        return div().id("frame").size_full().child(content);
    };
    window.set_client_inset(theme::FRAME_SHADOW);
    let tokens = cx.tokens();
    let zones = if window.is_resizable() { zones(window.viewport_size(), tiling) } else { Vec::new() };
    div()
        .id("frame")
        .relative()
        .size_full()
        .bg(transparent_black())
        .when(!tiling.top, |frame| frame.pt(theme::FRAME_SHADOW))
        .when(!tiling.bottom, |frame| frame.pb(theme::FRAME_SHADOW))
        .when(!tiling.left, |frame| frame.pl(theme::FRAME_SHADOW))
        .when(!tiling.right, |frame| frame.pr(theme::FRAME_SHADOW))
        .child(
            div()
                .size_full()
                .map(|edge| round(edge, tiling, theme::FRAME_RADIUS))
                .border_color(tokens.edge)
                .when(!tiling.top, |edge| edge.border_t(theme::FRAME_BORDER))
                .when(!tiling.bottom, |edge| edge.border_b(theme::FRAME_BORDER))
                .when(!tiling.left, |edge| edge.border_l(theme::FRAME_BORDER))
                .when(!tiling.right, |edge| edge.border_r(theme::FRAME_BORDER))
                .when(!tiling.is_tiled(), |edge| {
                    let shadow = tokens.shadow.opacity(if window.is_window_active() { 1. } else { 0.6 });
                    edge.shadow(theme::depth::frame(shadow))
                })
                .child(content),
        )
        .children(zones.into_iter().map(|(edge, bounds)| {
            div()
                .id(edge_name(edge))
                .absolute()
                .left(bounds.left())
                .top(bounds.top())
                .w(bounds.size.width)
                .h(bounds.size.height)
                .occlude()
                .cursor(match edge {
                    ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                    ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
                })
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.start_window_resize(edge))
        }))
}

/// Where in a window `size` large each edge not tiled takes the drags that resize by it: the
/// shadow's margin and a little of the content, the corners a square of that reach, as they are
/// harder to hit.
fn zones(size: Size<Pixels>, tiling: Tiling) -> Vec<(ResizeEdge, Bounds<Pixels>)> {
    let reach = theme::FRAME_SHADOW * 1.5;
    // Along an edge, between the corners there are; the whole length where both ends are tiled.
    let span = |start_tiled: bool, end_tiled: bool, length: Pixels| {
        let start = if start_tiled { Pixels::ZERO } else { reach };
        let end = if end_tiled { length } else { length - reach };
        (start, end - start)
    };
    let (width, height) = (size.width, size.height);
    let mut zones = Vec::new();
    let mut zone = |edge, x, y, w, h| zones.push((edge, Bounds::new(point(x, y), gpui::size(w, h))));
    let (x, w) = span(tiling.left, tiling.right, width);
    if !tiling.top {
        zone(ResizeEdge::Top, x, Pixels::ZERO, w, reach);
    }
    if !tiling.bottom {
        zone(ResizeEdge::Bottom, x, height - reach, w, reach);
    }
    let (y, h) = span(tiling.top, tiling.bottom, height);
    if !tiling.left {
        zone(ResizeEdge::Left, Pixels::ZERO, y, reach, h);
    }
    if !tiling.right {
        zone(ResizeEdge::Right, width - reach, y, reach, h);
    }
    let corners = [
        (ResizeEdge::TopLeft, tiling.top || tiling.left, Pixels::ZERO, Pixels::ZERO),
        (ResizeEdge::TopRight, tiling.top || tiling.right, width - reach, Pixels::ZERO),
        (ResizeEdge::BottomLeft, tiling.bottom || tiling.left, Pixels::ZERO, height - reach),
        (ResizeEdge::BottomRight, tiling.bottom || tiling.right, width - reach, height - reach),
    ];
    for (edge, tiled, x, y) in corners {
        if !tiled {
            zone(edge, x, y, reach, reach);
        }
    }
    zones
}

fn edge_name(edge: ResizeEdge) -> &'static str {
    match edge {
        ResizeEdge::Top => "resize-top",
        ResizeEdge::Bottom => "resize-bottom",
        ResizeEdge::Left => "resize-left",
        ResizeEdge::Right => "resize-right",
        ResizeEdge::TopLeft => "resize-top-left",
        ResizeEdge::TopRight => "resize-top-right",
        ResizeEdge::BottomLeft => "resize-bottom-left",
        ResizeEdge::BottomRight => "resize-bottom-right",
    }
}

/// `element` with `radius` at the corners not tiled.
fn round<E: Styled + FluentBuilder>(element: E, tiling: Tiling, radius: Pixels) -> E {
    element
        .when(!(tiling.top || tiling.left), |element| element.rounded_tl(radius))
        .when(!(tiling.top || tiling.right), |element| element.rounded_tr(radius))
        .when(!(tiling.bottom || tiling.left), |element| element.rounded_bl(radius))
        .when(!(tiling.bottom || tiling.right), |element| element.rounded_br(radius))
}

/// The content's corners, rounded inside the frame's where the app draws it: what fills the
/// content, its background and a dialog's scrim, is rounded with them.
pub fn rounded<E: Styled + FluentBuilder>(content: E, window: &Window) -> E {
    match window.window_decorations() {
        Decorations::Client { tiling } => round(content, tiling, theme::FRAME_RADIUS - theme::FRAME_BORDER),
        Decorations::Server => content,
    }
}

/// The title bar: its content, which ends `TITLE_INSET` from its edges, and the window's own
/// buttons where the app draws them. Pressed and moved, it moves the window; clicked twice, it
/// zooms it.
#[derive(IntoElement)]
pub struct TitleBar {
    content: Option<AnyElement>,
}

impl TitleBar {
    pub fn new(content: Option<AnyElement>) -> Self {
        Self { content }
    }
}

/// Whether the pointer was pressed on the title bar and not yet released, when moving it moves
/// the window.
#[derive(Default)]
struct Pressed(bool);

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pressed = window.use_keyed_state("title-bar", cx, |_, _| Pressed::default());
        let client = matches!(window.window_decorations(), Decorations::Client { .. });
        let controls = window.window_controls();
        let [left, right] = window_buttons(window, cx);
        let start = if cfg!(target_os = "macos") && !window.is_fullscreen() { theme::TRAFFIC_LIGHTS } else { theme::TITLE_INSET };
        let zooms = controls.maximize && window.is_resizable();

        h_flex()
            .id("title-bar")
            .flex_none()
            .w_full()
            .h(theme::TITLE_BAR)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, window.listener_for(&pressed, |pressed, _, _, _| pressed.0 = true))
            .on_mouse_up(MouseButton::Left, window.listener_for(&pressed, |pressed, _, _, _| pressed.0 = false))
            .on_mouse_down_out(window.listener_for(&pressed, |pressed, _, _, _| pressed.0 = false))
            .on_mouse_move(window.listener_for(&pressed, |pressed, _, window, _| {
                if pressed.0 {
                    pressed.0 = false;
                    window.start_window_move();
                }
            }))
            // On Windows the system zooms the window itself, the bar being its caption.
            .on_click(move |event, window, _| {
                if event.click_count() == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else if cfg!(target_os = "linux") && zooms {
                        window.zoom_window();
                    }
                }
            })
            .when(cfg!(target_os = "linux") && client && controls.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |event, window, _| window.show_window_menu(event.position))
            })
            .children((!left.is_empty()).then(|| buttons(left, window, cx)))
            .child(h_flex().flex_1().min_w_0().h_full().pl(start).pr(theme::TITLE_INSET).children(self.content))
            .children((!right.is_empty()).then(|| buttons(right, window, cx)))
    }
}

/// The window's buttons on the title bar's left and right, where the app draws them: on Linux
/// those the desktop sets, in its order, which the window manager offers for the window.
#[cfg(target_os = "linux")]
fn window_buttons(window: &Window, cx: &App) -> [Vec<WindowButton>; 2] {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return [Vec::new(), Vec::new()];
    }
    let controls = window.window_controls();
    let layout = cx.button_layout().unwrap_or_else(gpui::WindowButtonLayout::linux_default);
    let offered = |buttons: &[Option<WindowButton>]| {
        buttons
            .iter()
            .flatten()
            .copied()
            .filter(|button| match button {
                WindowButton::Minimize => controls.minimize,
                WindowButton::Maximize => controls.maximize,
                WindowButton::Close => true,
            })
            .collect()
    };
    [offered(&layout.left), offered(&layout.right)]
}

/// On Windows its three, on the right.
#[cfg(target_os = "windows")]
fn window_buttons(_: &Window, _: &App) -> [Vec<WindowButton>; 2] {
    [Vec::new(), vec![WindowButton::Minimize, WindowButton::Maximize, WindowButton::Close]]
}

/// On macOS the system's own, in the title bar's corner.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn window_buttons(_: &Window, _: &App) -> [Vec<WindowButton>; 2] {
    [Vec::new(), Vec::new()]
}

/// The window's buttons, as the system's: on Windows full height and caption width, which the
/// system clicks itself; on Linux small circles, clicked by the app.
fn buttons(buttons: Vec<WindowButton>, window: &Window, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    let windows = cfg!(target_os = "windows");
    let maximized = window.is_maximized();
    h_flex().h_full().when(!windows, |side| side.gap_2().px(theme::TITLE_INSET)).children(buttons.into_iter().map(|button| {
        let glyph = match button {
            WindowButton::Minimize => Glyph::Minimize,
            WindowButton::Maximize if maximized => Glyph::Restore,
            WindowButton::Maximize => Glyph::Maximize,
            WindowButton::Close => Glyph::Close,
        };
        let close = button == WindowButton::Close;
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
                        WindowButton::Minimize => WindowControlArea::Min,
                        WindowButton::Maximize => WindowControlArea::Max,
                        WindowButton::Close => WindowControlArea::Close,
                    };
                    control
                        .w(theme::CAPTION_BUTTON)
                        .h_full()
                        .window_control_area(area)
                        .when(close, |control| control.hover(|style| style.bg(tokens.danger).text_color(tokens.on_primary)))
                        .when(!close, |control| control.hover(|style| style.bg(tokens.inset).text_color(tokens.text)))
                } else {
                    control
                        .size(theme::WINDOW_BUTTON)
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
                                WindowButton::Minimize => window.minimize_window(),
                                WindowButton::Maximize => window.zoom_window(),
                                WindowButton::Close => window.remove_window(),
                            }
                        })
                }
            })
            .child(Icon::new(glyph).size(if windows { theme::ICON } else { theme::ICON_SM }))
    }))
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    #[test]
    fn the_margin_resizes_by_the_edges_and_corners_not_tiled() {
        let window = size(theme::step(100.), theme::step(75.));
        let edges = |tiling| zones(window, tiling).into_iter().map(|(edge, _)| edge).collect::<Vec<_>>();
        assert_eq!(edges(Tiling::default()).len(), 8);
        let top = Tiling { top: true, ..Tiling::default() };
        assert_eq!(edges(top), [ResizeEdge::Bottom, ResizeEdge::Left, ResizeEdge::Right, ResizeEdge::BottomLeft, ResizeEdge::BottomRight]);
        assert!(edges(Tiling::tiled()).is_empty());
    }

    #[test]
    fn a_side_reaches_into_a_tiled_corner() {
        let top = Tiling { top: true, ..Tiling::default() };
        let (_, left) =
            zones(size(theme::step(100.), theme::step(75.)), top).into_iter().find(|(edge, _)| *edge == ResizeEdge::Left).unwrap();
        assert_eq!(left.top(), Pixels::ZERO);
        assert_eq!(left.bottom(), theme::step(75.) - theme::FRAME_SHADOW * 1.5);
    }
}
