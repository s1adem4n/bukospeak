//! Every size of the app, on one scale: Tailwind's, a step being 4 px. Spacing in a screen is
//! GPUI's Tailwind methods (`gap_3`, `p_4`), which are this scale; the sizes of things are the
//! tokens here, each so many steps or made of others. Nothing else makes a size: clippy refuses
//! `px` outside the theme, and GPUI's own type, radius and shadow presets everywhere.
#![allow(clippy::disallowed_methods, reason = "the scale is where sizes are made")]

use gpui::{Pixels, Size, Styled, px};

/// `steps` steps of 4 px; half and quarter steps for the fine parts of a control.
pub const fn step(steps: f32) -> Pixels {
    px(steps * 4.)
}

/// A hairline: the edge of a surface and the line between rows.
pub const HAIRLINE: Pixels = px(1.);

/// A size of type and the line it sits on; lines are whole steps, so text stacks on the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: Pixels,
    pub line: Pixels,
}

/// The type scale: desktop sizes, read at arm's length.
pub const TEXT_XS: Type = Type { size: px(11.), line: step(4.) };
pub const TEXT_SM: Type = Type { size: px(12.), line: step(4.) };
pub const TEXT: Type = Type { size: px(13.), line: step(5.) };
pub const TEXT_MD: Type = Type { size: px(14.), line: step(5.) };
pub const TEXT_XL: Type = Type { size: px(20.), line: step(7.) };
/// Initials on large avatars.
pub const TEXT_2XL: Type = Type { size: px(24.), line: step(8.) };
pub const TEXT_3XL: Type = Type { size: px(32.), line: step(10.) };

/// Sets text in a size of the type scale, on its line.
pub trait Typeset: Styled + Sized {
    fn typeset(self, kind: Type) -> Self {
        self.text_size(kind.size).line_height(kind.line)
    }
}

impl<E: Styled> Typeset for E {}

/// Icons: in small controls and beside small text, in controls, and in large ones.
pub const ICON_SM: Pixels = step(3.5);
pub const ICON: Pixels = step(4.);
pub const ICON_LG: Pixels = step(4.5);

/// Controls and fields, small, medium and large; a row of a list is as high as a large one.
pub const CONTROL_SM: Pixels = step(7.);
pub const CONTROL: Pixels = step(9.);
pub const CONTROL_LG: Pixels = step(11.);
pub const ROW: Pixels = CONTROL_LG;
/// How far a control's content lies inside its edge, by its size.
pub const CONTROL_SM_INSET: Pixels = step(2.5);
pub const CONTROL_INSET: Pixels = step(3.5);
pub const CONTROL_LG_INSET: Pixels = step(4.);

/// The space inside a panel and a menu, around what is set into them, in steps.
const PANEL_PAD_STEPS: f32 = 1.;
const MENU_PAD_STEPS: f32 = 1.;
pub const PANEL_PAD: Pixels = step(PANEL_PAD_STEPS);
pub const MENU_PAD: Pixels = step(MENU_PAD_STEPS);
/// Where a panel's content starts: a row's inset inside the panel's padding, so labels line up
/// with titles above the panel.
pub const PANEL_INSET: Pixels = step(4.);
/// The space between the window's edge and what is on it.
pub const GUTTER: Pixels = step(3.5);

/// Corners. A control's is its own; what holds controls takes theirs plus the space between, so
/// nested edges run alike.
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

/// An avatar's size, and the type of the initial on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarSize {
    pub size: Pixels,
    pub initial: Type,
    /// The badge on its edge: muted, reconnecting.
    pub badge: Pixels,
}

/// Avatars: on a small tile, on a middling one, and on a large one or alone.
pub const AVATAR: AvatarSize = AvatarSize { size: step(12.), initial: TEXT_XL, badge: step(5.) };
pub const AVATAR_LG: AvatarSize = AvatarSize { size: step(16.), initial: TEXT_2XL, badge: step(6.) };
pub const AVATAR_XL: AvatarSize = AvatarSize { size: step(20.), initial: TEXT_3XL, badge: step(7.) };
/// The edge a badge is cut out of the avatar with.
pub const BADGE_EDGE: Pixels = step(0.5);
/// The edge a speaker's tile takes on.
pub const SPEAKING_EDGE: Pixels = step(0.5);

/// How the members sit on the stage, by how many there are: the tiles' height, at most their
/// width, their corners, the avatar on them and the type of the name beneath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seats {
    pub columns: usize,
    pub height: Pixels,
    pub width: Pixels,
    pub radius: Pixels,
    pub avatar: AvatarSize,
    /// The space between the avatar and the name.
    pub gap: Pixels,
    pub name: Type,
}

/// Up to two, side by side as if facing each other.
pub const SEATS_TWO: Seats =
    Seats { columns: 2, height: step(56.), width: step(44.), radius: RADIUS_TILE, avatar: AVATAR_XL, gap: step(4.), name: TEXT_MD };
/// Three or four, two by two.
pub const SEATS_FOUR: Seats =
    Seats { columns: 2, height: step(38.), width: step(44.), radius: RADIUS_TILE, avatar: AVATAR_LG, gap: step(3.), name: TEXT_MD };
/// More, in threes.
pub const SEATS_MANY: Seats =
    Seats { columns: 3, height: step(28.), width: step(32.), radius: RADIUS_TILE_SM, avatar: AVATAR, gap: step(2.), name: TEXT };

/// The window's own title bar, drawn with the content, and how far its content lies off its edges.
pub const TITLE_BAR: Pixels = step(10.);
pub const TITLE_INSET: Pixels = GUTTER;
/// The blank left of the back chevron's stroke in its icon, by which the chevron hangs out to
/// line up with what is below.
pub const BACK_BEARING: Pixels = step(1.5);
/// On macOS: the window's buttons, `TRAFFIC_LIGHT` high, centered in the title bar as far from
/// its left edge as from its top, and the room they take with the space after them.
pub const TRAFFIC_LIGHT: Pixels = step(3.5);
pub const TRAFFIC_LIGHTS: Pixels = step(20.);
/// On Windows: the width of each of the window's buttons, as the system draws its own.
pub const CAPTION_BUTTON: Pixels = px(46.);
/// On Linux and macOS without the system's: the window's round buttons.
pub const WINDOW_BUTTON: Pixels = step(5.5);
/// On Linux, where the app draws the window's edge: its corners' radius, its border, and the
/// margin around it its shadow falls into, which also takes the drags that resize the window.
pub const FRAME_RADIUS: Pixels = step(2.5);
pub const FRAME_BORDER: Pixels = HAIRLINE;
pub const FRAME_SHADOW: Pixels = step(2.5);

/// The windows: the main one, the least it shrinks to, and the update window before it.
pub const WINDOW: (Pixels, Pixels) = (step(95.), step(140.));
pub const WINDOW_MIN: (Pixels, Pixels) = (step(80.), step(110.));
pub const UPDATE_WINDOW: (Pixels, Pixels) = (step(75.), step(80.));

/// How far a field's text lies inside its edge, and its cursor's width.
pub const FIELD_INSET: Pixels = step(3.);
pub const CURSOR: Pixels = step(0.375);
/// A switch's track and the knob in it.
pub const SWITCH_WIDTH: Pixels = step(9.);
pub const SWITCH_HEIGHT: Pixels = step(5.);
pub const SWITCH_KNOB: Pixels = step(4.);
/// A slider: the height that takes its clicks, its rail and its knob.
pub const SLIDER: Pixels = step(5.);
pub const SLIDER_RAIL: Pixels = step(1.);
pub const SLIDER_KNOB: Pixels = step(4.);
/// A chip's line: keys on a button.
pub const KEYCAP: Pixels = step(4.);
/// A status's dot and the halo around it.
pub const DOT: Pixels = step(1.75);
pub const DOT_HALO: Pixels = step(0.75);
/// A menu: the least width of one, its items, and its distance from what opened it.
pub const MENU: Pixels = step(56.);
pub const MENU_ITEM: Pixels = CONTROL_SM;
pub const MENU_GAP: Pixels = step(1.);
/// A tooltip's distance from the pointer.
pub const TOOLTIP_OFFSET: Pixels = step(2.5);
/// The volume's value after its slider, wide enough for the loudest.
pub const VOLUME_VALUE: Pixels = step(10.);
/// The welcome's column, and how far below the title bar it hangs at most.
pub const WELCOME: Pixels = step(74.);
pub const WELCOME_TOP: Pixels = step(20.);
/// The app's mark in the welcome and the update window.
pub const MARK: Pixels = step(20.);
/// The microphone's level: a rail as wide as a short value, as thick as the volume's, and how many
/// steps it fills in.
pub const LEVEL: Size<Pixels> = Size { width: step(12.), height: SLIDER_RAIL };
pub const LEVEL_STEPS: usize = 24;
/// The update window's column, and the bar of how far its download is.
pub const UPDATE: Pixels = step(56.);
pub const PROGRESS: Pixels = step(1.5);

/// How far something rises as it comes on, and a tile while its member speaks.
pub const RISE: Pixels = step(2.);
pub const LIFT: Pixels = step(0.5);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every size on the grid: whole steps, or the half and quarter steps of a control's parts.
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
