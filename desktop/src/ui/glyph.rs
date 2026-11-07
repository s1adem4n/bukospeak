use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! glyphs {
    ($($glyph:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Glyph {
            $($glyph,)*
        }

        impl Glyph {
            const ALL: &[Glyph] = &[$(Glyph::$glyph,)*];

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
    Warning => "triangle-alert",
    Users => "users",
    You => "user",
    Reconnecting => "wifi-off",
    Edit => "pencil",
    Choose => "chevrons-up-down",
    Download => "download",
    Loading => "loader-circle",
    Check => "check",
    Minimize => "minus",
    Close => "x",
}

impl Glyph {
    pub fn path(self) -> SharedString {
        self.file().0.into()
    }
}

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
