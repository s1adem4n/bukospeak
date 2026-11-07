pub mod depth;
mod fonts;
mod motion;
mod scale;

pub use fonts::{MONO, SANS};
pub use motion::*;
pub use scale::*;

use gpui::{App, Background, Global, Hsla, Window, WindowAppearance, hsla, linear_color_stop, linear_gradient, rgb, rgba};

pub fn install(cx: &mut App) {
    fonts::install(cx);
    let dark = is_dark(cx.window_appearance());
    cx.set_global(Palette { light: Tokens::light(), dark: Tokens::dark(), is_dark: dark, forced: None });
}

#[cfg(feature = "harness")]
pub fn force(dark: bool, cx: &mut App) {
    let palette = cx.global_mut::<Palette>();
    (palette.forced, palette.is_dark) = (Some(dark), dark);
    cx.refresh_windows();
}

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

struct Palette {
    light: Tokens,
    dark: Tokens,
    is_dark: bool,
    forced: Option<bool>,
}

impl Global for Palette {}

pub trait ActiveTokens {
    fn tokens(&self) -> &Tokens;
}

impl ActiveTokens for App {
    fn tokens(&self) -> &Tokens {
        let palette = self.global::<Palette>();
        if palette.is_dark { &palette.dark } else { &palette.light }
    }
}

#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    pub base: Hsla,
    pub raised: Hsla,
    pub inset: Hsla,
    pub hover: Hsla,
    pub floating: Hsla,

    pub hairline: Hsla,
    pub edge: Hsla,
    pub highlight: Hsla,
    pub sheen: Hsla,
    pub focus: Hsla,
    pub shadow: Hsla,
    pub shadow_soft: Hsla,

    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,

    pub control: [Hsla; 2],
    pub control_hover: [Hsla; 2],
    pub primary: [Hsla; 2],
    pub primary_hover: [Hsla; 2],
    pub primary_edge: Hsla,
    pub on_primary: Hsla,

    pub tile: [Hsla; 2],
    pub seat: Hsla,

    pub speaking: Hsla,
    pub danger: Hsla,
    pub danger_fill: [Hsla; 2],
    pub danger_fill_hover: [Hsla; 2],
    pub danger_edge: Hsla,
    pub on_danger: Hsla,
    pub warning: Hsla,
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
            on_danger: rgb(0xffffff).into(),
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
            on_danger: rgb(0xffffff).into(),
            warning: rgb(0xb45309).into(),
            selection: rgba(0x2563eb40).into(),
        }
    }
}

pub fn gradient([top, bottom]: [Hsla; 2]) -> Background {
    linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bottom, 1.))
}

const HUES: [f32; 8] = [212., 258., 328., 18., 40., 152., 184., 352.];

pub fn avatar(name: &str, dark: bool) -> (Hsla, Hsla) {
    // FNV-1a: stable across runs and systems, unlike the standard library's hasher.
    let hash = name.bytes().fold(0x811c_9dc5_u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    let hue = HUES[hash as usize % HUES.len()] / 360.;
    if dark { (hsla(hue, 0.3, 0.24, 1.), hsla(hue, 0.7, 0.82, 1.)) } else { (hsla(hue, 0.65, 0.92, 1.), hsla(hue, 0.5, 0.36, 1.)) }
}

pub const RECONNECTING_OPACITY: f32 = 0.45;
pub const DISABLED_OPACITY: f32 = 0.5;
pub const WAITING_OPACITY: f32 = 0.4;
