//! The app's look: Inter on every system, the colors of `assets/theme.json` for light and dark,
//! and the sizes, type and motion kept here by hand.

use std::borrow::Cow;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeSet};
use gpui_kit::{App, Hsla, Pixels, hsla, px};

/// Inter in the three weights the app sets, so text looks the same everywhere and a weight is
/// a weight, not whatever a system's variable font makes of it.
const FONTS: [&[u8]; 3] = [
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
];
const THEMES: &str = include_str!("../../assets/theme.json");

/// Adds the fonts and makes the app's themes the component theme's light and dark; the
/// window's appearance then picks one.
pub fn install(cx: &mut App) {
    cx.text_system().add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect()).expect("Inter loads");
    let set: ThemeSet = serde_json::from_str(THEMES).expect("the themes are valid");
    Theme::update(cx, |theme| {
        // A focused field shows by its border; a ring around it is one line too many.
        theme.focus_ring = false;
        for config in set.themes {
            let config = Rc::new(config);
            if config.mode.is_dark() {
                theme.dark_theme = config;
            } else {
                theme.light_theme = config;
            }
        }
    });
}

/// Someone speaking, and a call that is connected.
pub fn speaking(dark: bool) -> Hsla {
    if dark { hsla(151. / 360., 0.65, 0.54, 1.) } else { hsla(148. / 360., 0.75, 0.36, 1.) }
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
pub const TEXT_LG: Pixels = px(15.);
pub const TEXT_XL: Pixels = px(20.);

/// The space between the window's edge and its content, which every screen lines up on.
pub const GUTTER: Pixels = px(12.);
/// How far a control's edge lies outside its icon: pulled out by it, a control's icon lines up
/// on the gutter.
pub const CONTROL_INSET: Pixels = px(8.);
/// Between a field and the button below it, and between fields of one group.
pub const BEFORE_ACTION: Pixels = px(16.);
pub const BETWEEN_FIELDS: Pixels = px(8.);

/// The window's own title bar, drawn with the content; it keeps the gutter on the left itself.
pub const TITLE_BAR: Pixels = px(40.);
/// How far a row's content lies inside its edge.
pub const ROW_INSET: Pixels = px(8.);
/// A member in the list.
pub const ROW: Pixels = px(36.);
pub const ROW_AVATAR: Pixels = px(24.);
/// The speaking ring's width, and the gap between it and the avatar.
pub const RING: Pixels = px(2.);
/// The buttons of the call and of the title bar, and their icons.
pub const CONTROL: Pixels = px(32.);
pub const ICON: Pixels = px(16.);
/// A member's muted or reconnecting mark, quieter than a button's icon.
pub const MARK_ICON: Pixels = px(14.);
/// The main buttons and the text fields.
pub const FIELD: Pixels = px(36.);
/// The welcome's column, and the widest the settings grow.
pub const WELCOME: Pixels = px(280.);
pub const SETTINGS: Pixels = px(440.);
/// The microphone that welcomes, and one's own avatar after it.
pub const MARK: Pixels = px(112.);
pub const WELCOME_AVATAR: Pixels = px(64.);

/// The speaking ring, quick enough to follow speech.
pub const SPEAKING_FADE: Duration = Duration::from_millis(120);
/// Something coming onto the screen.
pub const APPEAR: Duration = Duration::from_millis(220);
/// How far a welcome step slides in from.
pub const SLIDE: Pixels = px(12.);
/// Faded while the connection is lost.
pub const RECONNECTING_OPACITY: f32 = 0.45;
