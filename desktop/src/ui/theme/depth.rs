#![allow(clippy::disallowed_methods, reason = "depth is where shadows are made")]

use gpui::{BoxShadow, Hsla, Pixels, px};

use crate::ui::theme::{FRAME_SHADOW, HAIRLINE, Tokens, step};

fn shadow(y: Pixels, blur: Pixels, color: Hsla) -> BoxShadow {
    BoxShadow::new(px(0.), y, color).blur_radius(blur)
}

fn edge_light(light: Hsla) -> BoxShadow {
    BoxShadow::new(px(0.), HAIRLINE, light).inset()
}

pub fn control(light: Hsla, tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(light), shadow(HAIRLINE, step(0.5), tokens.shadow)]
}

pub fn control_lg(light: Hsla, tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(light), shadow(HAIRLINE, step(0.5), tokens.shadow), shadow(step(2.), step(5.), tokens.shadow_soft)]
}

pub fn panel(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![shadow(HAIRLINE, step(0.5), tokens.shadow_soft)]
}

pub fn tile(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![edge_light(tokens.highlight), shadow(step(0.5), step(1.), tokens.shadow_soft), shadow(step(2.5), step(6.), tokens.shadow_soft)]
}

pub fn floating(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![shadow(step(0.5), step(1.5), tokens.shadow_soft), shadow(step(2.), step(6.), tokens.shadow_soft)]
}

pub fn frame(color: Hsla) -> Vec<BoxShadow> {
    vec![shadow(step(0.5), FRAME_SHADOW / 2., color)]
}

pub fn inner_edge(color: Hsla) -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.), px(0.), color).spread_radius(HAIRLINE).inset()]
}
