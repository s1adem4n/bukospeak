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

pub const CONTROL_SM: Pixels = step(7.);
pub const CONTROL: Pixels = step(9.);
pub const CONTROL_LG: Pixels = step(11.);
pub const ROW: Pixels = CONTROL_LG;
pub const CONTROL_SM_INSET: Pixels = step(2.5);
pub const CONTROL_INSET: Pixels = step(3.5);
pub const CONTROL_LG_INSET: Pixels = step(4.);

const PANEL_PAD_STEPS: f32 = 1.;
const MENU_PAD_STEPS: f32 = 1.;
pub const PANEL_PAD: Pixels = step(PANEL_PAD_STEPS);
pub const MENU_PAD: Pixels = step(MENU_PAD_STEPS);
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
pub const RADIUS_MENU: Pixels = step(RADIUS_CONTROL_SM_STEPS + MENU_PAD_STEPS);
pub const RADIUS_TILE: Pixels = step(5.);
pub const RADIUS_TILE_SM: Pixels = step(4.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarSize {
    pub size: Pixels,
    pub initial: Type,
    pub badge: Pixels,
}

pub const AVATAR: AvatarSize = AvatarSize { size: step(12.), initial: TEXT_XL, badge: step(5.) };
pub const AVATAR_LG: AvatarSize = AvatarSize { size: step(16.), initial: TEXT_2XL, badge: step(6.) };
pub const AVATAR_XL: AvatarSize = AvatarSize { size: step(20.), initial: TEXT_3XL, badge: step(7.) };
pub const BADGE_EDGE: Pixels = step(0.5);
pub const SPEAKING_EDGE: Pixels = step(0.5);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seats {
    pub columns: usize,
    pub height: Pixels,
    pub width: Pixels,
    pub radius: Pixels,
    pub avatar: AvatarSize,
    pub gap: Pixels,
    pub name: Type,
}

pub const SEATS_TWO: Seats =
    Seats { columns: 2, height: step(56.), width: step(44.), radius: RADIUS_TILE, avatar: AVATAR_XL, gap: step(4.), name: TEXT_MD };
pub const SEATS_FOUR: Seats =
    Seats { columns: 2, height: step(38.), width: step(44.), radius: RADIUS_TILE, avatar: AVATAR_LG, gap: step(3.), name: TEXT_MD };
pub const SEATS_MANY: Seats =
    Seats { columns: 3, height: step(28.), width: step(32.), radius: RADIUS_TILE_SM, avatar: AVATAR, gap: step(2.), name: TEXT };

pub const TITLE_BAR: Pixels = step(GUTTER_STEPS * 2. + ICON_STEPS);
pub const TITLE_INSET: Pixels = GUTTER;
pub const TRAFFIC_LIGHT: Pixels = step(3.5);
pub const TRAFFIC_LIGHTS: Pixels = step(20.);
// Windows draws its own caption buttons 46 px wide.
pub const CAPTION_BUTTON: Pixels = px(46.);
pub const WINDOW_BUTTON: Pixels = step(5.5);
pub const FRAME_RADIUS: Pixels = step(2.5);
pub const FRAME_BORDER: Pixels = HAIRLINE;
pub const FRAME_SHADOW: Pixels = step(2.5);

pub const WINDOW: (Pixels, Pixels) = (step(95.), step(140.));
pub const UPDATE_WINDOW: (Pixels, Pixels) = (step(75.), step(80.));

pub const FIELD_INSET: Pixels = step(3.);
pub const CURSOR: Pixels = step(0.375);
pub const SWITCH_WIDTH: Pixels = step(9.);
pub const SWITCH_HEIGHT: Pixels = step(5.);
pub const SWITCH_KNOB: Pixels = step(4.);
pub const SLIDER: Pixels = step(5.);
pub const SLIDER_RAIL: Pixels = step(1.);
pub const SLIDER_KNOB: Pixels = step(4.);
pub const KEYCAP: Pixels = step(4.);
pub const DOT: Pixels = step(1.75);
pub const DOT_HALO: Pixels = step(0.75);
pub const MENU: Pixels = step(56.);
pub const MENU_ITEM: Pixels = CONTROL_SM;
pub const MENU_SEPARATOR: Pixels = step(1.);
pub const MENU_RISE: Pixels = step(1.);
pub const TOOLTIP_OFFSET: Size<Pixels> = Size { width: step(2.), height: step(2.5) };
pub const TOOLTIP_INSET: Size<Pixels> = Size { width: step(2.), height: step(1.) };
pub const KEYCAP_INSET: Pixels = step(1.5);
pub const TILE_INSET: Pixels = step(3.);
pub const VOLUME_VALUE: Pixels = step(10.);
pub const WELCOME: Pixels = step(74.);
pub const WELCOME_TOP: Pixels = step(20.);
pub const MARK: Pixels = step(20.);
pub const LEVEL: Pixels = SLIDER_RAIL;
pub const LEVEL_STEPS: usize = 24;
pub const UPDATE: Pixels = step(56.);
pub const PROGRESS: Pixels = step(1.5);

pub const RISE: Pixels = step(2.);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_lie_on_the_grid() {
        let on_grid = |size: Pixels| (size.as_f32() * 2.).fract() == 0.;
        for size in [TEXT_XS, TEXT_SM, TEXT, TEXT_MD, TEXT_XL, TEXT_2XL, TEXT_3XL].map(|kind| kind.line) {
            assert_eq!(size.as_f32() % 4., 0., "a line of {size:?} is off the grid");
        }
        let sizes = [ICON_SM, ICON, ICON_LG, CONTROL_SM, CONTROL, CONTROL_LG, RADIUS_PANEL, RADIUS_MENU, AVATAR_XL.size];
        assert!(sizes.into_iter().all(on_grid));
    }

    #[test]
    fn nested_corners_take_the_space_between() {
        assert_eq!(RADIUS_PANEL - PANEL_PAD, RADIUS_CONTROL);
        assert_eq!(RADIUS_MENU - MENU_PAD, RADIUS_CONTROL_SM);
    }
}
