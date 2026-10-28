//! How loud the microphone is, as a row of bars that light up from the left.

use std::time::Duration;

use bukospeak_core::Meter;
use gpui_kit::component::h_flex;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Task, Window, div};

use crate::ui::theme::{self, ActiveTokens};

/// How often the bars look at the meter: as often as a level is seen to move.
const LOOK_EVERY: Duration = Duration::from_millis(33);

/// The bars of a meter, redrawn only when how many are lit changes.
pub struct InputLevel {
    meter: Meter,
    lit: usize,
    _looking: Task<()>,
}

impl InputLevel {
    pub fn new(meter: Meter, cx: &mut Context<Self>) -> Self {
        let looking = cx.spawn(async move |level, cx| {
            loop {
                cx.background_executor().timer(LOOK_EVERY).await;
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

/// How many bars a level lights.
fn lit(level: f32) -> usize {
    (level * theme::LEVEL_BARS as f32).round() as usize
}

impl Render for InputLevel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        h_flex().flex_none().gap(theme::LEVEL_GAP).children((0..theme::LEVEL_BARS).map(|bar| {
            div().w(theme::LEVEL_BAR_WIDTH).h(theme::LEVEL_BAR_HEIGHT).rounded_full().bg(if bar < self.lit {
                tokens.speaking
            } else {
                tokens.hover
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_lights_none_and_full_scale_all() {
        assert_eq!(lit(0.0), 0);
        assert_eq!(lit(0.5), 3);
        assert_eq!(lit(1.0), theme::LEVEL_BARS);
    }
}
