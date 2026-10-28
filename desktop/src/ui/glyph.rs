//! The icons the app shows, from the Lucide set gpui-kit bundles, and the assets that hold
//! them: only these and the components' own, not all of Lucide.

use std::borrow::Cow;

use gpui_kit::assets::{IconName, icon_assets};
use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};

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
    Edit,
    /// A value chosen from a menu.
    Choose,
}

impl From<Glyph> for Icon {
    fn from(glyph: Glyph) -> Self {
        Icon::new(match glyph {
            Glyph::Mic => IconName::Mic,
            Glyph::MicOff => IconName::MicOff,
            Glyph::PhoneOff => IconName::PhoneOff,
            Glyph::Settings => IconName::Settings,
            Glyph::Back => IconName::ChevronLeft,
            Glyph::Warning => IconName::TriangleAlert,
            Glyph::Users => IconName::Users,
            Glyph::Reconnecting => IconName::WifiOff,
            Glyph::Edit => IconName::Pencil,
            Glyph::Choose => IconName::ChevronsUpDown,
        })
    }
}

icon_assets!(Glyphs, [Mic, MicOff, PhoneOff, Settings, ChevronLeft, TriangleAlert, Users, WifiOff, Pencil, ChevronsUpDown]);

/// The glyphs above and the icons gpui-kit's components draw themselves.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match Glyphs.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(Glyphs.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
