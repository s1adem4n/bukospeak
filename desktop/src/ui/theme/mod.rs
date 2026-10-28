//! The app's look: Geist on every system, the colors of [`Tokens`] for light and dark, and the
//! sizes, type and motion kept here by hand. gpui-component's theme is made from the tokens, so
//! its fields, menus, switches and sliders look like the rest.

mod component;
mod fonts;

pub use fonts::MONO;

use std::time::Duration;

use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::{App, Global, Hsla, Pixels, hsla, px, rgb, rgba};

/// Adds the fonts, keeps the tokens of both appearances and makes gpui-component's light and
/// dark themes of them; the window's appearance then picks one.
pub fn install(cx: &mut App) {
    fonts::install(cx);
    let palette = Palette { light: Tokens::light(), dark: Tokens::dark() };
    let (light, dark) = (component::theme(&palette.light), component::theme(&palette.dark));
    cx.set_global(palette);
    Theme::update(cx, |theme| {
        // A focused field shows by its border; a ring around it is one line too many.
        theme.focus_ring = false;
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
}

/// The tokens of both appearances.
struct Palette {
    light: Tokens,
    dark: Tokens,
}

impl Global for Palette {}

/// The tokens of the appearance the window has now.
pub trait ActiveTokens {
    fn tokens(&self) -> &Tokens;
}

impl ActiveTokens for App {
    fn tokens(&self) -> &Tokens {
        let palette = self.global::<Palette>();
        if self.theme().is_dark() { &palette.dark } else { &palette.light }
    }
}

/// The colors of one appearance, by what they are for.
pub struct Tokens {
    pub dark: bool,
    /// The window, beneath everything.
    pub base: Hsla,
    /// A panel on the window.
    pub raised: Hsla,
    /// Something set into a panel: a field, a hovered row, a fingerprint.
    pub inset: Hsla,
    /// A hovered control on an inset surface, a switch's track.
    pub hover: Hsla,
    /// Menus and tooltips, which float above the panels.
    pub floating: Hsla,

    /// The edge of a panel and the lines between its rows.
    pub hairline: Hsla,
    /// The edge of a control.
    pub edge: Hsla,
    /// The light that catches a raised control's top edge, and a colored one's.
    pub highlight: Hsla,
    pub sheen: Hsla,
    /// A focused field's edge.
    pub focus: Hsla,
    /// The shadow beneath raised controls and floating surfaces.
    pub shadow: Hsla,
    /// What darkens the window behind a dialog.
    pub scrim: Hsla,

    pub text: Hsla,
    /// Labels and values that aren't the point of a line.
    pub text_muted: Hsla,
    /// What is there for the record: the server, a count, a placeholder.
    pub text_faint: Hsla,

    /// A raised control's fill, top to bottom, and on hover.
    pub control: [Hsla; 2],
    pub control_hover: [Hsla; 2],
    /// The one action that moves things on: its fill top to bottom, its edge, its text.
    pub primary: [Hsla; 2],
    pub primary_hover: [Hsla; 2],
    pub primary_edge: Hsla,
    pub on_primary: Hsla,

    /// Someone speaking, a call that is connected.
    pub speaking: Hsla,
    /// What ends something, and what is off and wants attention.
    pub danger: Hsla,
    pub danger_hover: Hsla,
    /// What is filled red, top to bottom, and on hover, and its edge.
    pub danger_fill: [Hsla; 2],
    pub danger_fill_hover: [Hsla; 2],
    pub danger_edge: Hsla,
    pub warning: Hsla,
    /// What selected text lies on.
    pub selection: Hsla,
}

impl Tokens {
    fn dark() -> Self {
        Self {
            dark: true,
            base: rgb(0x0b0b0d).into(),
            raised: rgb(0x141417).into(),
            inset: rgb(0x1b1b1f).into(),
            hover: rgb(0x26262b).into(),
            floating: rgb(0x1c1c20).into(),
            hairline: rgba(0xffffff0e).into(),
            edge: rgba(0xffffff16).into(),
            highlight: rgba(0xffffff1a).into(),
            sheen: rgba(0xffffff47).into(),
            focus: rgba(0xffffff3d).into(),
            shadow: rgba(0x00000066).into(),
            scrim: rgba(0x00000099).into(),
            text: rgb(0xededef).into(),
            text_muted: rgb(0x9d9da6).into(),
            text_faint: rgb(0x63636c).into(),
            control: [rgb(0x202024).into(), rgb(0x1a1a1e).into()],
            control_hover: [rgb(0x26262b).into(), rgb(0x1f1f23).into()],
            primary: [rgb(0xfafafa).into(), rgb(0xdcdce0).into()],
            primary_hover: [rgb(0xffffff).into(), rgb(0xe8e8eb).into()],
            primary_edge: rgb(0xbdbdc3).into(),
            on_primary: rgb(0x0b0b0d).into(),
            speaking: rgb(0x3dd68c).into(),
            danger: rgb(0xe5484d).into(),
            danger_hover: rgb(0xec5d5e).into(),
            danger_fill: [rgb(0xee5f63).into(), rgb(0xdb3d42).into()],
            danger_fill_hover: [rgb(0xf3706f).into(), rgb(0xe24a4e).into()],
            danger_edge: rgb(0xa8292e).into(),
            warning: rgb(0xf5b544).into(),
            selection: rgba(0x3b82f659).into(),
        }
    }

    fn light() -> Self {
        Self {
            dark: false,
            base: rgb(0xf2f2f4).into(),
            raised: rgb(0xffffff).into(),
            inset: rgb(0xf5f5f7).into(),
            hover: rgb(0xe9e9ec).into(),
            floating: rgb(0xffffff).into(),
            hairline: rgba(0x0000000f).into(),
            edge: rgba(0x0000001a).into(),
            highlight: rgba(0xffffffff).into(),
            sheen: rgba(0xffffff4d).into(),
            focus: rgba(0x00000047).into(),
            shadow: rgba(0x0000001a).into(),
            scrim: rgba(0x00000040).into(),
            text: rgb(0x18181b).into(),
            text_muted: rgb(0x6b6b73).into(),
            text_faint: rgb(0x9b9ba3).into(),
            control: [rgb(0xffffff).into(), rgb(0xf7f7f8).into()],
            control_hover: [rgb(0xfafafa).into(), rgb(0xf0f0f2).into()],
            primary: [rgb(0x2a2a2f).into(), rgb(0x18181b).into()],
            primary_hover: [rgb(0x36363c).into(), rgb(0x232327).into()],
            primary_edge: rgb(0x0b0b0d).into(),
            on_primary: rgb(0xfafafa).into(),
            speaking: rgb(0x18a058).into(),
            danger: rgb(0xe5484d).into(),
            danger_hover: rgb(0xdc3e42).into(),
            danger_fill: [rgb(0xee5f63).into(), rgb(0xdb3d42).into()],
            danger_fill_hover: [rgb(0xf26c6f).into(), rgb(0xe0464b).into()],
            danger_edge: rgb(0xc2272c).into(),
            warning: rgb(0xb45309).into(),
            selection: rgba(0x2563eb40).into(),
        }
    }
}

/// The hues a member's avatar takes, one by their name, so a face is told apart at a glance.
const HUES: [f32; 8] = [212., 258., 328., 18., 40., 152., 184., 352.];

/// A member's avatar's background and initial, from their name; the same name always gets the
/// same colors.
pub fn avatar(name: &str, dark: bool) -> (Hsla, Hsla) {
    // FNV-1a: stable across runs and systems, unlike the standard library's hasher.
    let hash = name.bytes().fold(0x811c_9dc5_u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    let hue = HUES[hash as usize % HUES.len()] / 360.;
    if dark { (hsla(hue, 0.3, 0.24, 1.), hsla(hue, 0.7, 0.82, 1.)) } else { (hsla(hue, 0.65, 0.92, 1.), hsla(hue, 0.5, 0.36, 1.)) }
}

/// The type scale: desktop sizes, read at arm's length.
pub const TEXT_XS: Pixels = px(11.);
pub const TEXT_SM: Pixels = px(12.);
pub const TEXT: Pixels = px(13.);
pub const TEXT_LG: Pixels = px(14.);
pub const TEXT_XL: Pixels = px(20.);

/// Corners, each the one around it less the space between: nested edges run alike.
pub const RADIUS_PANEL: Pixels = px(14.);
pub const RADIUS_CONTROL: Pixels = px(10.);
pub const RADIUS_SMALL: Pixels = px(8.);
pub const RADIUS_TINY: Pixels = px(5.);

/// The space between the window's edge and its panels, and between the panels.
pub const GAP: Pixels = px(8.);
/// How far a panel's content lies inside its edge: rows' labels, a head's title.
pub const PANEL_INSET: Pixels = px(14.);
/// The space inside a panel around the controls and rows set into it.
pub const PANEL_PAD: Pixels = px(4.);

/// The window's own title bar, drawn with the content, and how far gpui-component's keeps its
/// content off its left edge.
pub const TITLE_BAR: Pixels = px(40.);
pub const TITLE_INSET: Pixels = px(12.);
/// A panel's head, and a row of a list.
pub const HEAD: Pixels = px(44.);
pub const ROW: Pixels = px(42.);
/// A member in the channel.
pub const MEMBER: Pixels = px(40.);
pub const MEMBER_AVATAR: Pixels = px(26.);
/// The speaking ring's width, and the gap between it and the avatar.
pub const RING: Pixels = px(2.);
/// The controls: the main ones, the small ones in bars and rows, and the fields.
pub const CONTROL: Pixels = px(36.);
pub const CONTROL_SM: Pixels = px(28.);
pub const FIELD: Pixels = px(38.);
pub const FIELD_SM: Pixels = px(30.);
pub const ICON_LG: Pixels = px(18.);
pub const ICON: Pixels = px(16.);
pub const ICON_SM: Pixels = px(14.);
/// A chip's line: a count, and keys on a button.
pub const CHIP: Pixels = px(18.);
pub const KEYCAP: Pixels = px(16.);
/// The dock's line of how the call is.
pub const STATUS: Pixels = px(30.);
/// A dialog: its width, and its space inside, the corner around its controls' less it.
pub const DIALOG: Pixels = px(340.);
pub const DIALOG_PAD: Pixels = px(6.);
pub const RADIUS_DIALOG: Pixels = px(16.);
/// A menu that chooses a device.
pub const MENU: Pixels = px(220.);
/// The volume's slider, and its value after it.
pub const VOLUME: Pixels = px(170.);
pub const VOLUME_VALUE: Pixels = px(40.);
/// The welcome's column, and its dots of where in it one is.
pub const WELCOME: Pixels = px(300.);
pub const STEP_DOT: Pixels = px(6.);
pub const STEP_DOT_CURRENT: Pixels = px(18.);
/// The app's mark in the title bar and in the welcome, and one's own avatar there.
pub const MARK_SM: Pixels = px(20.);
pub const MARK: Pixels = px(96.);
pub const AVATAR_LG: Pixels = px(40.);
pub const AVATAR_XL: Pixels = px(56.);

/// The speaking ring, quick enough to follow speech.
pub const SPEAKING_FADE: Duration = Duration::from_millis(120);
/// Something coming onto the screen.
pub const APPEAR: Duration = Duration::from_millis(220);
/// How far a welcome step slides in from.
pub const SLIDE: Pixels = px(12.);
/// Faded while the connection is lost.
pub const RECONNECTING_OPACITY: f32 = 0.45;
/// Faded while a control can't be used.
pub const DISABLED_OPACITY: f32 = 0.5;
