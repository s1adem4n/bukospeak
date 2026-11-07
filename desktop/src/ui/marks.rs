#[cfg(feature = "harness")]
use gpui::{App, Bounds, Global, Pixels, canvas};
use gpui::{ParentElement, SharedString, Styled};

#[cfg(feature = "harness")]
#[derive(Default)]
pub struct Marks(pub Vec<(SharedString, Bounds<Pixels>)>);

#[cfg(feature = "harness")]
impl Global for Marks {}

#[cfg(feature = "harness")]
impl Marks {
    pub fn clear(cx: &mut App) {
        cx.default_global::<Self>().0.clear();
    }
}

pub trait Mark: ParentElement + Styled + Sized {
    // Positions the element relative, so only an element in the flow can be marked.
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
