use std::time::Duration;

use gpui::SpringConfig;

pub const BLINK: Duration = Duration::from_millis(530);
pub const SPIN: Duration = Duration::from_millis(800);
pub const POP: Duration = Duration::from_millis(140);
pub const APPEAR: Duration = Duration::from_millis(240);

pub const SPEAK_ON: SpringConfig = SpringConfig::new(900., 60., 1.);
// Slower, so the gaps between words don't flicker the light.
pub const SPEAK_OFF: SpringConfig = SpringConfig::new(110., 21., 1.);
