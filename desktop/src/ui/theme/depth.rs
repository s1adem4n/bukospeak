//! Depth: the light on a surface's top edge and the shadows beneath it, by how far the surface
//! stands off what is under it. Every shadow of the app is one of these.
#![allow(clippy::disallowed_methods, reason = "depth is where shadows are made")]

use gpui::{BoxShadow, Hsla, Pixels, px};

use crate::ui::theme::{FRAME_SHADOW, HAIRLINE, Tokens, step};

fn shadow(y: Pixels, blur: Pixels, color: Hsla) -> BoxShadow {
    BoxShadow::new(px(0.), y, color).blur_radius(blur)
}

/// The light along a surface's top edge.
fn edge_light(light: Hsla) -> BoxShadow {
    BoxShadow::new(px(0.), HAIRLINE, light).inset()
}

/// A control lifted off the surface it sits on, its top edge caught by `light`.
pub fn control(light: Hsla, tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(light), shadow(HAIRLINE, step(0.5), tokens.shadow)]
}

/// A large control, standing further off: the dock's.
pub fn control_lg(light: Hsla, tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(light), shadow(HAIRLINE, step(0.5), tokens.shadow), shadow(step(2.), step(5.), tokens.shadow_soft)]
}

/// A panel on the window.
pub fn panel(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![shadow(HAIRLINE, step(0.5), tokens.shadow_soft)]
}

/// A member's tile on the stage, well off the window.
pub fn tile(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(tokens.highlight), shadow(step(0.5), step(1.), tokens.shadow_soft), shadow(step(2.5), step(6.), tokens.shadow_soft)]
}

/// Menus and tooltips, floating above everything.
pub fn floating(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![shadow(step(0.5), step(1.5), tokens.shadow_soft), shadow(step(2.), step(6.), tokens.shadow_soft)]
}

/// A dialog, above a scrim.
pub fn overlay(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(tokens.highlight), shadow(step(1.), step(3.), tokens.shadow_soft), shadow(step(6.), step(12.), tokens.shadow)]
}

/// The window's edge where the app draws it, in `color`, dimmed while the window is inactive.
pub fn frame(color: Hsla) -> Vec<BoxShadow> {
    vec![shadow(step(0.5), FRAME_SHADOW / 2., color)]
}

/// An edge inside a circle, so a dark avatar doesn't melt into what it is on.
pub fn inner_edge(color: Hsla) -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.), px(0.), color).spread_radius(HAIRLINE).inset()]
}
