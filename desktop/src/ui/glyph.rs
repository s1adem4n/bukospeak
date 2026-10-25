//! The icons the app shows, from the Lucide set gpui-kit bundles.

use gpui_kit::component::Icon;

#[derive(Clone, Copy, Debug)]
pub enum Glyph {
    Mic,
    MicOff,
    PhoneOff,
    Settings,
    Back,
    Warning,
    Users,
    Reconnecting,
}

impl From<Glyph> for Icon {
    fn from(glyph: Glyph) -> Self {
        Icon::empty().path(match glyph {
            Glyph::Mic => "icons/mic.svg",
            Glyph::MicOff => "icons/mic-off.svg",
            Glyph::PhoneOff => "icons/phone-off.svg",
            Glyph::Settings => "icons/settings.svg",
            Glyph::Back => "icons/arrow-left.svg",
            Glyph::Warning => "icons/triangle-alert.svg",
            Glyph::Users => "icons/users.svg",
            Glyph::Reconnecting => "icons/wifi-off.svg",
        })
    }
}
