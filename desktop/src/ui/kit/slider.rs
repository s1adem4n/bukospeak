use gpui::{
    App, Bounds, Context, Corners, CursorStyle, DispatchPhase, Element, ElementId, EventEmitter, FocusHandle, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Render, Role, StatefulInteractiveElement, Style, Styled, Window, actions, div, fill, point,
    relative, size,
};

use crate::ui::marks::Mark;
use crate::ui::theme::{self, ActiveTokens};

actions!(slider, [Decrease, Increase]);

const CONTEXT: &str = "Slider";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", Decrease, Some(CONTEXT)),
        KeyBinding::new("down", Decrease, Some(CONTEXT)),
        KeyBinding::new("right", Increase, Some(CONTEXT)),
        KeyBinding::new("up", Increase, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderEvent {
    pub value: f32,
    pub done: bool,
}

pub struct Slider {
    value: f32,
    least: f32,
    most: f32,
    step: f32,
    label: &'static str,
    focus: FocusHandle,
    dragging: bool,
}

impl EventEmitter<SliderEvent> for Slider {}

impl Slider {
    pub fn new(label: &'static str, value: f32, least: f32, most: f32, step: f32, cx: &mut Context<Self>) -> Self {
        Self { value: snap(value, least, most, step), least, most, step, label, focus: cx.focus_handle().tab_stop(true), dragging: false }
    }

    fn fraction(&self) -> f32 {
        (self.value - self.least) / (self.most - self.least)
    }

    fn set(&mut self, value: f32, done: bool, cx: &mut Context<Self>) {
        let value = snap(value, self.least, self.most, self.step);
        if value != self.value || done {
            self.value = value;
            cx.emit(SliderEvent { value, done });
            cx.notify();
        }
    }

    fn set_at(&mut self, x: Pixels, bounds: Bounds<Pixels>, done: bool, cx: &mut Context<Self>) {
        let along = (x - bounds.left()) / bounds.size.width;
        self.set(self.least + along.clamp(0., 1.) * (self.most - self.least), done, cx);
    }

    fn decrease(&mut self, _: &Decrease, _: &mut Window, cx: &mut Context<Self>) {
        self.set(self.value - self.step, true, cx);
    }

    fn increase(&mut self, _: &Increase, _: &mut Window, cx: &mut Context<Self>) {
        self.set(self.value + self.step, true, cx);
    }
}

fn snap(value: f32, least: f32, most: f32, step: f32) -> f32 {
    (least + ((value - least) / step).round() * step).clamp(least, most)
}

impl Render for Slider {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        div()
            .id("slider")
            .mark(self.label)
            .role(Role::Slider)
            .aria_label(self.label)
            .aria_numeric_value(f64::from(self.value))
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::decrease))
            .on_action(cx.listener(Self::increase))
            .w_full()
            .h(theme::slider::HEIGHT)
            .rounded_full()
            .border_1()
            .border_color(tokens.base.opacity(0.))
            .focus_visible(|style| style.border_color(tokens.focus))
            .child(Rail { slider: cx.entity() })
    }
}

struct Rail {
    slider: gpui::Entity<Slider>,
}

impl IntoElement for Rail {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Rail {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style { size: size(relative(1.).into(), relative(1.).into()), ..Style::default() };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Hitbox {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let tokens = cx.tokens();
        let knob = theme::slider::KNOB;
        // The knob's middle stays a half-width in from the ends, so it never reaches past them.
        let rail = Bounds::new(
            point(bounds.left() + knob / 2., bounds.center().y - theme::slider::RAIL / 2.),
            size(bounds.size.width - knob, theme::slider::RAIL),
        );
        let x = rail.left() + rail.size.width * self.slider.read(cx).fraction();
        let round = Corners::all(theme::slider::RAIL / 2.);
        window.paint_quad(fill(rail, tokens.hover).corner_radii(round));
        window.paint_quad(fill(Bounds::from_corners(rail.origin, point(x, rail.bottom())), tokens.text).corner_radii(round));
        let knob_bounds = Bounds::new(point(x - knob / 2., bounds.center().y - knob / 2.), size(knob, knob));
        window.paint_quad(
            fill(knob_bounds, tokens.primary[0])
                .corner_radii(Corners::all(knob / 2.))
                .border_widths(theme::HAIRLINE)
                .border_color(tokens.edge),
        );
        window.set_cursor_style(CursorStyle::PointingHand, hitbox);

        let slider = self.slider.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event({
            let (slider, hitbox) = (slider.clone(), hitbox.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && hitbox.is_hovered(window) {
                    window.capture_pointer(hitbox.id);
                    slider.update(cx, |slider, cx| {
                        slider.dragging = true;
                        slider.set_at(event.position.x, rail, false, cx);
                    });
                }
            }
        });
        window.on_mouse_event({
            let (slider, hitbox) = (slider.clone(), hitbox.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) && slider.read(cx).dragging {
                    slider.update(cx, |slider, cx| slider.set_at(event.position.x, rail, false, cx));
                }
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && slider.read(cx).dragging {
                slider.update(cx, |slider, cx| {
                    slider.dragging = false;
                    slider.set_at(event.position.x, rail, true, cx);
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_snaps_to_the_nearest_step_within_the_ends() {
        assert_eq!(snap(42., 0., 200., 5.), 40.);
        assert_eq!(snap(43., 0., 200., 5.), 45.);
        assert_eq!(snap(-3., 0., 200., 5.), 0.);
        assert_eq!(snap(260., 0., 200., 5.), 200.);
    }
}
