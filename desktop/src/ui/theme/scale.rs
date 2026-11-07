#![allow(clippy::disallowed_methods, reason = "the scale is where sizes are made")]

use gpui::{Pixels, Size, Styled, px};

pub const fn step(steps: f32) -> Pixels {
    px(steps * 4.)
}

pub const HAIRLINE: Pixels = px(1.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: Pixels,
    pub line: Pixels,
}

pub const TEXT_XS: Type = Type { size: px(11.), line: step(4.) };
pub const TEXT_SM: Type = Type { size: px(12.), line: step(4.) };
pub const TEXT: Type = Type { size: px(13.), line: step(5.) };
pub const TEXT_MD: Type = Type { size: px(14.), line: step(5.) };
pub const TEXT_XL: Type = Type { size: px(20.), line: step(7.) };
pub const TEXT_2XL: Type = Type { size: px(24.), line: step(8.) };
pub const TEXT_3XL: Type = Type { size: px(32.), line: step(10.) };

pub trait Typeset: Styled + Sized {
    fn typeset(self, kind: Type) -> Self {
        self.text_size(kind.size).line_height(kind.line)
    }
}

impl<E: Styled> Typeset for E {}

pub const ICON_SM: Pixels = step(3.5);
const ICON_STEPS: f32 = 4.;
pub const ICON: Pixels = step(ICON_STEPS);
pub const ICON_LG: Pixels = step(4.5);

const CONTROL_SM_STEPS: f32 = 7.;
pub const CONTROL_SM: Pixels = step(CONTROL_SM_STEPS);
pub const CONTROL: Pixels = step(9.);
pub const CONTROL_LG: Pixels = step(11.);
pub const ROW: Pixels = CONTROL_LG;
pub const CONTROL_SM_INSET: Pixels = step(2.5);
pub const CONTROL_INSET: Pixels = step(3.5);
pub const CONTROL_LG_INSET: Pixels = step(4.);

const PANEL_PAD_STEPS: f32 = 1.;
pub const PANEL_PAD: Pixels = step(PANEL_PAD_STEPS);
pub const PANEL_INSET: Pixels = step(4.);
const GUTTER_STEPS: f32 = 4.;
pub const GUTTER: Pixels = step(GUTTER_STEPS);
pub const SECTION: Pixels = step(6.);
pub const GAP: Pixels = step(3.);
pub const CAPTION: Pixels = step(2.);
pub const INLINE: Pixels = step(2.);
pub const TIGHT: Pixels = step(1.5);
pub const LINE: Pixels = step(1.);

const RADIUS_CONTROL_SM_STEPS: f32 = 2.;
const RADIUS_CONTROL_STEPS: f32 = 2.5;
const RADIUS_CONTROL_LG_STEPS: f32 = 3.5;
pub const RADIUS_XS: Pixels = step(1.);
pub const RADIUS_CONTROL_SM: Pixels = step(RADIUS_CONTROL_SM_STEPS);
pub const RADIUS_CONTROL: Pixels = step(RADIUS_CONTROL_STEPS);
pub const RADIUS_CONTROL_LG: Pixels = step(RADIUS_CONTROL_LG_STEPS);
pub const RADIUS_PANEL: Pixels = step(RADIUS_CONTROL_STEPS + PANEL_PAD_STEPS);

pub const RISE: Pixels = step(2.);
pub const MARK: Pixels = step(20.);

pub mod avatar {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Size {
        pub size: Pixels,
        pub initial: Type,
        pub badge: Pixels,
    }

    pub const MD: Size = Size { size: step(12.), initial: TEXT_XL, badge: step(5.) };
    pub const LG: Size = Size { size: step(16.), initial: TEXT_2XL, badge: step(6.) };
    pub const XL: Size = Size { size: step(20.), initial: TEXT_3XL, badge: step(7.) };
    pub const BADGE_EDGE: Pixels = step(0.5);
}

pub mod app_icon {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Size {
        pub size: Pixels,
        pub radius: Pixels,
        pub initial: Type,
    }

    pub const MD: Size = Size { size: step(6.), radius: step(1.5), initial: TEXT_SM };
    pub const SM: Size = Size { size: step(5.), radius: step(1.25), initial: TEXT_XS };
}

pub mod seats {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Layout {
        pub columns: usize,
        pub height: Pixels,
        pub width: Pixels,
        pub radius: Pixels,
        pub avatar: avatar::Size,
        pub gap: Pixels,
        pub name: Type,
    }

    const RADIUS: Pixels = step(5.);
    const RADIUS_SM: Pixels = step(4.);
    pub const TWO: Layout =
        Layout { columns: 2, height: step(56.), width: step(44.), radius: RADIUS, avatar: avatar::XL, gap: step(4.), name: TEXT_MD };
    pub const FOUR: Layout =
        Layout { columns: 2, height: step(38.), width: step(44.), radius: RADIUS, avatar: avatar::LG, gap: step(3.), name: TEXT_MD };
    pub const MANY: Layout =
        Layout { columns: 3, height: step(28.), width: step(32.), radius: RADIUS_SM, avatar: avatar::MD, gap: step(2.), name: TEXT };
    pub const SPEAKING_EDGE: Pixels = step(0.5);
    pub const INSET: Pixels = step(3.);
}

pub mod window {
    use super::*;

    pub const MAIN: (Pixels, Pixels) = (step(95.), step(140.));
    pub const UPDATE: (Pixels, Pixels) = (step(75.), step(80.));
    pub const TITLE_BAR: Pixels = step(GUTTER_STEPS * 2. + ICON_STEPS);
    pub const TITLE_INSET: Pixels = GUTTER;
    pub const TRAFFIC_LIGHT: Pixels = step(3.5);
    pub const TRAFFIC_LIGHTS: Pixels = step(20.);
    // Windows draws its own caption buttons 46 px wide.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub const CAPTION_BUTTON: Pixels = px(46.);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub const BUTTON: Pixels = step(5.5);
    pub const FRAME_RADIUS: Pixels = step(2.5);
    pub const FRAME_BORDER: Pixels = HAIRLINE;
    pub const FRAME_SHADOW: Pixels = step(2.5);
}

pub mod field {
    use super::*;

    pub const INSET: Pixels = step(3.);
    pub const CURSOR: Pixels = step(0.375);
}

pub mod switch {
    use super::*;

    pub const WIDTH: Pixels = step(9.);
    pub const HEIGHT: Pixels = step(5.);
    pub const KNOB: Pixels = step(4.);
}

pub mod slider {
    use super::*;

    pub const HEIGHT: Pixels = step(5.);
    pub const RAIL: Pixels = step(1.);
    pub const KNOB: Pixels = step(4.);
    pub const VALUE: Pixels = step(10.);
}

pub mod level {
    use super::*;

    pub const HEIGHT: Pixels = slider::RAIL;
    pub const STEPS: usize = 24;
}

pub mod keycap {
    use super::*;

    pub const HEIGHT: Pixels = step(4.);
    pub const INSET: Pixels = step(1.5);
}

pub mod dot {
    use super::*;

    pub const SIZE: Pixels = step(1.75);
    pub const HALO: Pixels = step(0.75);
}

pub mod progress {
    use super::*;

    pub const HEIGHT: Pixels = step(1.5);
}

pub mod menu {
    use super::*;

    const PAD_STEPS: f32 = 1.;
    const ITEMS_BEFORE_SCROLLING: f32 = 12.;
    pub const PAD: Pixels = step(PAD_STEPS);
    pub const RADIUS: Pixels = step(RADIUS_CONTROL_SM_STEPS + PAD_STEPS);
    pub const WIDTH: Pixels = step(56.);
    pub const ITEM: Pixels = CONTROL_SM;
    pub const LIST: Pixels = step(CONTROL_SM_STEPS * ITEMS_BEFORE_SCROLLING);
    pub const SEPARATOR: Pixels = step(1.);
    pub const RISE: Pixels = step(1.);
}

pub mod tooltip {
    use super::*;

    pub const OFFSET: Size<Pixels> = Size { width: step(2.), height: step(2.5) };
    pub const INSET: Size<Pixels> = Size { width: step(2.), height: step(1.) };
}

pub mod welcome {
    use super::*;

    pub const WIDTH: Pixels = step(74.);
    pub const TOP: Pixels = step(20.);
}

pub mod update {
    use super::*;

    pub const WIDTH: Pixels = step(56.);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_lie_on_the_grid() {
        let on_grid = |size: Pixels| (size.as_f32() * 2.).fract() == 0.;
        for size in [TEXT_XS, TEXT_SM, TEXT, TEXT_MD, TEXT_XL, TEXT_2XL, TEXT_3XL].map(|kind| kind.line) {
            assert_eq!(size.as_f32() % 4., 0., "a line of {size:?} is off the grid");
        }
        let sizes = [ICON_SM, ICON, ICON_LG, CONTROL_SM, CONTROL, CONTROL_LG, RADIUS_PANEL, menu::RADIUS, avatar::XL.size];
        assert!(sizes.into_iter().all(on_grid));
    }

    #[test]
    fn nested_corners_take_the_space_between() {
        assert_eq!(RADIUS_PANEL - PANEL_PAD, RADIUS_CONTROL);
        assert_eq!(menu::RADIUS - menu::PAD, RADIUS_CONTROL_SM);
    }
}
