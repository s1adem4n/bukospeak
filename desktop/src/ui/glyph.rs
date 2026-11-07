//! The icons the app shows, from Lucide (`assets/icons/LICENSE.txt`), and the assets that hold
//! them.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Each glyph and the file of its icon, in `assets/icons`.
macro_rules! glyphs {
    ($($(#[$doc:meta])* $glyph:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Glyph {
            $($(#[$doc])* $glyph,)*
        }

        impl Glyph {
            const ALL: &[Glyph] = &[$(Glyph::$glyph,)*];

            /// Where the icon lies among the assets, and its SVG.
            fn file(self) -> (&'static str, &'static [u8]) {
                match self {
                    $(Glyph::$glyph => (
                        concat!("icons/", $file, ".svg"),
                        include_bytes!(concat!("../../assets/icons/", $file, ".svg")),
                    ),)*
                }
            }
        }
    };
}

glyphs! {
    Mic => "mic",
    MicOff => "mic-off",
    PhoneOff => "phone-off",
    Settings => "settings",
    Back => "chevron-left",
    Warning => "triangle-alert",
    Users => "users",
    /// One's own avatar before the server named one.
    You => "user",
    Reconnecting => "wifi-off",
    Edit => "pencil",
    /// A value chosen from a menu.
    Choose => "chevrons-up-down",
    Download => "download",
    /// Something under way, turning.
    Loading => "loader-circle",
    /// A menu's item that is chosen.
    Check => "check",
    /// The window's buttons, where the app draws them.
    Minimize => "minus",
    Maximize => "square",
    Restore => "copy",
    Close => "x",
}

impl Glyph {
    pub fn path(self) -> SharedString {
        self.file().0.into()
    }
}

/// The glyphs' icons, inside the binary.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let file = Glyph::ALL.iter().map(|glyph| glyph.file()).find(|(at, _)| *at == path);
        Ok(file.map(|(_, svg)| Cow::Borrowed(svg)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Glyph::ALL.iter().map(|glyph| glyph.file().0).filter(|at| at.starts_with(path)).map(SharedString::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_loads_from_its_path() {
        for glyph in Glyph::ALL {
            let svg = Assets.load(&glyph.path()).unwrap().expect("the glyph's icon is among the assets");
            assert!(svg.starts_with(b"<svg"), "{glyph:?}");
        }
        assert_eq!(Assets.list("icons/").unwrap().len(), Glyph::ALL.len());
    }
}
