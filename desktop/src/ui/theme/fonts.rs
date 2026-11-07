// Built in, so text looks the same on every system and a weight is a weight, not
// whatever a system's variable font makes of it.

use std::borrow::Cow;

use gpui::App;

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

const FONTS: [&[u8]; 5] = [
    include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Medium.ttf"),
];

pub fn install(cx: &mut App) {
    cx.text_system().add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect()).expect("Geist loads");
}
