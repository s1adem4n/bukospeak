//! The app's look: Geist on every system, the colors of [`Tokens`] for light and dark, the one
//! scale every size is on, the depth of surfaces and how things move, all kept here by hand.

pub mod depth;
mod fonts;
mod motion;
mod scale;

pub use fonts::{MONO, SANS};
pub use motion::*;
pub use scale::*;

use gpui::{App, Background, Global, Hsla, Window, WindowAppearance, hsla, linear_color_stop, linear_gradient, rgb, rgba};

/// Adds the fonts and keeps the tokens of both appearances, the system's one active.
pub fn install(cx: &mut App) {
    fonts::install(cx);
    let dark = is_dark(cx.window_appearance());
    cx.set_global(Palette { light: Tokens::light(), dark: Tokens::dark(), is_dark: dark, forced: None });
}

/// Draws every window light or dark from now on, whatever the system's mode, as the harness
/// shows both.
#[cfg(feature = "harness")]
pub fn force(dark: bool, cx: &mut App) {
    let palette = cx.global_mut::<Palette>();
    (palette.forced, palette.is_dark) = (Some(dark), dark);
    cx.refresh_windows();
}

/// Takes on the window's appearance, which follows the system's light or dark mode; every
/// window is drawn anew once it changes.
pub fn follow(window: &Window, cx: &mut App) {
    let palette = cx.global_mut::<Palette>();
    let dark = palette.forced.unwrap_or_else(|| is_dark(window.appearance()));
    if palette.is_dark != dark {
        palette.is_dark = dark;
        cx.refresh_windows();
    }
}

fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// The tokens of both appearances, and which is active.
struct Palette {
    light: Tokens,
    dark: Tokens,
    is_dark: bool,
    /// The appearance the harness draws in, whatever the system's.
    forced: Option<bool>,
}

impl Global for Palette {}

/// The tokens of the active appearance.
pub trait ActiveTokens {
    fn tokens(&self) -> &Tokens;
}

impl ActiveTokens for App {
    fn tokens(&self) -> &Tokens {
        let palette = self.global::<Palette>();
        if palette.is_dark { &palette.dark } else { &palette.light }
    }
}

/// The colors of one appearance, by what they are for.
#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    /// The window, beneath everything.
    pub base: Hsla,
    /// A panel on the window.
    pub raised: Hsla,
    /// Something set into a panel: a field, a hovered row.
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
    /// The shadow close beneath a raised control, and the wide soft one beneath what stands
    /// further off.
    pub shadow: Hsla,
    pub shadow_soft: Hsla,

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

    /// A member's tile, top to bottom.
    pub tile: [Hsla; 2],
    /// The dashed edge of a place on the stage nobody sits in.
    pub seat: Hsla,

    /// Someone speaking, a call that is connected.
    pub speaking: Hsla,
    /// What ends something, and what is off and wants attention.
    pub danger: Hsla,
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
            shadow_soft: rgba(0x00000073).into(),
            text: rgb(0xededef).into(),
            text_muted: rgb(0x9d9da6).into(),
            text_faint: rgb(0x63636c).into(),
            control: [rgb(0x202024).into(), rgb(0x1a1a1e).into()],
            control_hover: [rgb(0x26262b).into(), rgb(0x1f1f23).into()],
            primary: [rgb(0xfafafa).into(), rgb(0xdcdce0).into()],
            primary_hover: [rgb(0xffffff).into(), rgb(0xe8e8eb).into()],
            primary_edge: rgb(0xbdbdc3).into(),
            on_primary: rgb(0x0b0b0d).into(),
            tile: [rgb(0x1a1a1e).into(), rgb(0x141417).into()],
            seat: rgba(0xffffff1f).into(),
            speaking: rgb(0x3dd68c).into(),
            danger: rgb(0xe5484d).into(),
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
            shadow_soft: rgba(0x00000012).into(),
            text: rgb(0x18181b).into(),
            text_muted: rgb(0x6b6b73).into(),
            text_faint: rgb(0x9b9ba3).into(),
            control: [rgb(0xffffff).into(), rgb(0xf7f7f8).into()],
            control_hover: [rgb(0xfafafa).into(), rgb(0xf0f0f2).into()],
            primary: [rgb(0x2a2a2f).into(), rgb(0x18181b).into()],
            primary_hover: [rgb(0x36363c).into(), rgb(0x232327).into()],
            primary_edge: rgb(0x0b0b0d).into(),
            on_primary: rgb(0xfafafa).into(),
            tile: [rgb(0xffffff).into(), rgb(0xfafafb).into()],
            seat: rgba(0x00000024).into(),
            speaking: rgb(0x18a058).into(),
            danger: rgb(0xe5484d).into(),
            danger_fill: [rgb(0xee5f63).into(), rgb(0xdb3d42).into()],
            danger_fill_hover: [rgb(0xf26c6f).into(), rgb(0xe0464b).into()],
            danger_edge: rgb(0xc2272c).into(),
            warning: rgb(0xb45309).into(),
            selection: rgba(0x2563eb40).into(),
        }
    }
}

/// A fill from its top color to its bottom one.
pub fn gradient([top, bottom]: [Hsla; 2]) -> Background {
    linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bottom, 1.))
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

/// Faded while the connection is lost.
pub const RECONNECTING_OPACITY: f32 = 0.45;
/// Faded while a control can't be used.
pub const DISABLED_OPACITY: f32 = 0.5;
/// Faded: one's own place on the stage before joining.
pub const WAITING_OPACITY: f32 = 0.4;
