use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, Decorations, Div, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, RenderOnce, Size,
    Stateful, StatefulInteractiveElement, Styled, Tiling, TitlebarOptions, Window, WindowControlArea, WindowDecorations, WindowOptions,
    div, point, transparent_black,
};

use crate::ui::kit::h_flex;
use crate::ui::theme::{self, ActiveTokens};

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod caption;

pub fn options(size: Size<Pixels>, cx: &App) -> WindowOptions {
    let light = (theme::window::TITLE_BAR - theme::window::TRAFFIC_LIGHT) / 2.;
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
    window.set_client_inset(theme::window::FRAME_SHADOW);
    let tokens = cx.tokens();
    div()
        .id("frame")
        .relative()
        .size_full()
        .bg(transparent_black())
        .when(!tiling.top, |frame| frame.pt(theme::window::FRAME_SHADOW))
        .when(!tiling.bottom, |frame| frame.pb(theme::window::FRAME_SHADOW))
        .when(!tiling.left, |frame| frame.pl(theme::window::FRAME_SHADOW))
        .when(!tiling.right, |frame| frame.pr(theme::window::FRAME_SHADOW))
        .child(
            div()
                .size_full()
                .map(|edge| round(edge, tiling, theme::window::FRAME_RADIUS))
                .border_color(tokens.edge)
                .when(!tiling.top, |edge| edge.border_t(theme::window::FRAME_BORDER))
                .when(!tiling.bottom, |edge| edge.border_b(theme::window::FRAME_BORDER))
                .when(!tiling.left, |edge| edge.border_l(theme::window::FRAME_BORDER))
                .when(!tiling.right, |edge| edge.border_r(theme::window::FRAME_BORDER))
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
        Decorations::Client { tiling } => round(content, tiling, theme::window::FRAME_RADIUS - theme::window::FRAME_BORDER),
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
        let [left, right] = caption_sides(window, cx);
        let start =
            if cfg!(target_os = "macos") && !window.is_fullscreen() { theme::window::TRAFFIC_LIGHTS } else { theme::window::TITLE_INSET };

        h_flex()
            .id("title-bar")
            .flex_none()
            .w_full()
            .h(theme::window::TITLE_BAR)
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
            .children(left)
            .child(h_flex().flex_1().min_w_0().h_full().pl(start).pr(theme::window::TITLE_INSET).children(self.content))
            .children(right)
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
use caption::sides as caption_sides;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn caption_sides(_: &Window, _: &App) -> [Option<AnyElement>; 2] {
    [None, None]
}
