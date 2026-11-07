use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, Decorations, Div, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, RenderOnce, Size,
    Stateful, StatefulInteractiveElement, Styled, Tiling, TitlebarOptions, Window, WindowControlArea, WindowDecorations, WindowOptions,
    div, point, transparent_black,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, h_flex};
use crate::ui::theme::{self, ActiveTokens};

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
        // What a window shows is laid out for its size.
        is_resizable: false,
        app_id: Some("bukospeak".into()),
        ..WindowOptions::default()
    }
}

pub fn frame(content: impl IntoElement, window: &mut Window, cx: &App) -> Stateful<Div> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        window.set_client_inset(Pixels::ZERO);
        return div().id("frame").size_full().child(content);
    };
    window.set_client_inset(theme::FRAME_SHADOW);
    let tokens = cx.tokens();
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
}

fn round<E: Styled + FluentBuilder>(element: E, tiling: Tiling, radius: Pixels) -> E {
    element
        .when(!(tiling.top || tiling.left), |element| element.rounded_tl(radius))
        .when(!(tiling.top || tiling.right), |element| element.rounded_tr(radius))
        .when(!(tiling.bottom || tiling.left), |element| element.rounded_bl(radius))
        .when(!(tiling.bottom || tiling.right), |element| element.rounded_br(radius))
}

pub fn rounded<E: Styled + FluentBuilder>(content: E, window: &Window) -> E {
    match window.window_decorations() {
        Decorations::Client { tiling } => round(content, tiling, theme::FRAME_RADIUS - theme::FRAME_BORDER),
        Decorations::Server => content,
    }
}

#[derive(IntoElement)]
pub struct TitleBar {
    content: Option<AnyElement>,
}

impl TitleBar {
    pub fn new(content: Option<AnyElement>) -> Self {
        Self { content }
    }
}

#[derive(Default)]
struct Pressed(bool);

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pressed = window.use_keyed_state("title-bar", cx, |_, _| Pressed::default());
        let client = matches!(window.window_decorations(), Decorations::Client { .. });
        let controls = window.window_controls();
        let [left, right] = window_buttons(window, cx);
        let start = if cfg!(target_os = "macos") && !window.is_fullscreen() { theme::TRAFFIC_LIGHTS } else { theme::TITLE_INSET };

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
            // Clicked twice, macOS does what its settings say, as for its own title bars.
            .when(cfg!(target_os = "macos"), |bar| {
                bar.on_click(|event, window, _| {
                    if event.click_count() == 2 {
                        window.titlebar_double_click();
                    }
                })
            })
            .when(cfg!(target_os = "linux") && client && controls.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |event, window, _| window.show_window_menu(event.position))
            })
            .children((!left.is_empty()).then(|| buttons(left, cx)))
            .child(h_flex().flex_1().min_w_0().h_full().pl(start).pr(theme::TITLE_INSET).children(self.content))
            .children((!right.is_empty()).then(|| buttons(right, cx)))
    }
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
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
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

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn window_buttons(_: &Window, _: &App) -> [Vec<Caption>; 2] {
    [Vec::new(), Vec::new()]
}

fn buttons(buttons: Vec<Caption>, cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    let windows = cfg!(target_os = "windows");
    h_flex().h_full().when(!windows, |side| side.gap(theme::INLINE).px(theme::TITLE_INSET)).children(buttons.into_iter().map(|button| {
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
                                Caption::Minimize => window.minimize_window(),
                                Caption::Close => window.remove_window(),
                            }
                        })
                }
            })
            .child(Icon::new(button.glyph()).size(theme::ICON_SM))
    }))
}
