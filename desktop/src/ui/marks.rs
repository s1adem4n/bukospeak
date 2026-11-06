//! Names on the elements the harness clicks and checks: in a build with the harness, each marked
//! element tells where it was drawn by its name; in the app, marking does nothing.

#[cfg(feature = "harness")]
use gpui::{App, Bounds, Global, Pixels, canvas};
use gpui::{ParentElement, SharedString, Styled};

/// Where the marked elements were drawn in the frame drawn last, by their names, in the order
/// they were drawn: a name drawn twice is there twice.
#[cfg(feature = "harness")]
#[derive(Default)]
pub struct Marks(pub Vec<(SharedString, Bounds<Pixels>)>);

#[cfg(feature = "harness")]
impl Global for Marks {}

#[cfg(feature = "harness")]
impl Marks {
    /// Forgets the marks of the frame before, for the next draw to tell them anew.
    pub fn clear(cx: &mut App) {
        cx.default_global::<Self>().0.clear();
    }
}

pub trait Mark: ParentElement + Styled + Sized {
    /// Names the element for the harness. It positions the element relative, so only an element
    /// in the flow takes a name.
    #[cfg(feature = "harness")]
    fn mark(self, name: impl Into<SharedString>) -> Self {
        let name = name.into();
        let recorder = canvas(
            move |bounds, _, cx| {
                // The harness installs the marks; a window drawn before has none to tell.
                if cx.has_global::<Marks>() {
                    cx.global_mut::<Marks>().0.push((name, bounds));
                }
            },
            |_, (), _, _| {},
        );
        self.relative().child(recorder.absolute().top_0().left_0().size_full())
    }

    #[cfg(not(feature = "harness"))]
    fn mark(self, _: impl Into<SharedString>) -> Self {
        self
    }
}

impl<E: ParentElement + Styled> Mark for E {}
