//! How things move: the few durations and springs of the app. What comes on eases out; what
//! follows a voice is a spring, quick to light and slow to fade, so a breath doesn't flicker it.

use std::time::Duration;

use gpui::SpringConfig;

/// How long a field's cursor shows and hides while it blinks.
pub const BLINK: Duration = Duration::from_millis(530);
/// One turn of a spinner.
pub const SPIN: Duration = Duration::from_millis(800);
/// A menu or a tooltip coming on.
pub const POP: Duration = Duration::from_millis(140);
/// A screen, a dialog or a member coming on.
pub const APPEAR: Duration = Duration::from_millis(240);

/// A member starting to speak: critically damped, settled in about a tenth of a second.
pub const SPEAK_ON: SpringConfig = SpringConfig::new(900., 60., 1.);
/// A member falling silent: slower, so the gaps between words don't flicker the light.
pub const SPEAK_OFF: SpringConfig = SpringConfig::new(110., 21., 1.);
