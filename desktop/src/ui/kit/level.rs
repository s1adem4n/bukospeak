use std::time::Duration;

use bukospeak_core::Meter;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Task, Window, div, relative};

use crate::ui::theme::{self, ActiveTokens};

const POLL_INTERVAL: Duration = Duration::from_millis(33);

pub struct InputLevel {
    meter: Meter,
    lit: usize,
    _looking: Task<()>,
}

impl InputLevel {
    pub fn new(meter: Meter, cx: &mut Context<Self>) -> Self {
        let looking = cx.spawn(async move |level, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let looked = level.update(cx, |level, cx| {
                    let lit = lit(level.meter.level());
                    if lit != level.lit {
                        level.lit = lit;
                        cx.notify();
                    }
                });
                if looked.is_err() {
                    break;
                }
            }
        });
        Self { lit: lit(meter.level()), meter, _looking: looking }
    }
}

fn lit(level: f32) -> usize {
    (level.clamp(0., 1.) * theme::level::STEPS as f32).round() as usize
}

impl Render for InputLevel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let filled = self.lit as f32 / theme::level::STEPS as f32;
        div()
            .flex_1()
            .h(theme::level::HEIGHT)
            .rounded_full()
            .bg(tokens.hover)
            .child(div().h_full().w(relative(filled)).rounded_full().bg(tokens.speaking))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_lights_none_and_full_scale_all() {
        assert_eq!(lit(0.0), 0);
        assert_eq!(lit(0.5), theme::level::STEPS / 2);
        assert_eq!(lit(1.0), theme::level::STEPS);
        assert_eq!(lit(2.0), theme::level::STEPS);
    }
}
