//! Voice activation: whether a captured frame is speech worth sending.
//!
//! A frame counts as voice when RNNoise finds it likely speech and it stands out from the noise
//! floor, the quietest the room was without speech in the last 1.5 s, both measured after noise
//! suppression: game audio from the room's speakers may sound like speech, but stays near the
//! floor. The gate holds open through short pauses, so words aren't cut apart, and the capture
//! sends a short pre-roll when it opens, so their onsets aren't either.

use bukospeak_proto::audio::Level;

/// RNNoise's voice probability above which a frame may be speech.
const VOICE_PROBABILITY: f32 = 0.6;

/// How far a frame must stand above the noise floor to be speech, in dB.
const ABOVE_FLOOR_DB: f32 = 9.0;

/// Frames quieter than this are never speech, however clean the room.
const QUIETEST_VOICE_DBOV: f32 = -55.0;

/// Frames the gate stays open after the last voice: 300 ms.
const HOLD: u32 = 30;

/// Frames before RNNoise tells speech once its input starts, as after the microphone opened or
/// was unmuted: its first frame comes out faded in by its window. From the next on its voice
/// probability and output are what they'd be had it run all along.
const WARM_UP: u32 = 1;

/// Frames the denoiser needs to learn the room's noise once its input starts: 200 ms. It lets
/// up to 8 dB more of it through for the first 100 ms, which would raise the floor; and noise
/// may seem speech to it at times, which only a floor heard before tells apart meanwhile.
pub const SETTLE: u32 = 20;

/// The floor is the quietest frame without speech in this many stretches of
/// [`FLOOR_STRETCH`] frames: 1.5 s, long enough to span the pauses between words. A stretch
/// without one keeps what it had, so sound that is all likely speech, like game music, doesn't
/// make the gate forget the room.
const FLOOR_STRETCHES: usize = 15;
const FLOOR_STRETCH: u32 = 10;

/// What to do with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not speech: don't send it.
    Quiet,
    /// Speech starts: send the pre-roll, then this frame.
    Opens,
    /// Speech goes on: send it.
    Speaks,
    /// The pause outlasted the hold: send it as the talkspurt's last frame.
    Closes,
}

pub struct Gate {
    /// Frames left before the gate closes, or None while it is closed.
    open_for: Option<u32>,
    /// Frames since the input started, up to [`SETTLE`].
    started: u32,
    /// The quietest frame without speech in each recent stretch, in dBov.
    quietest: [Option<f32>; FLOOR_STRETCHES],
    /// Whether the current stretch had a frame without speech yet.
    heard: bool,
    /// Frames the floor followed, which tells the stretch.
    frames: u32,
}

impl Default for Gate {
    fn default() -> Self {
        Self { open_for: None, started: 0, quietest: [None; FLOOR_STRETCHES], heard: false, frames: 0 }
    }
}

impl Gate {
    /// Decides on a frame by RNNoise's voice probability for it, its level after noise
    /// suppression, and whether it was captured with a `signal`, rather than digital silence.
    pub fn decide(&mut self, voice_probability: f32, level: Level, signal: bool) -> Decision {
        let dbov = -f32::from(level.minus_dbov());
        let likely_speech = voice_probability >= VOICE_PROBABILITY;
        // Digital silence captured is no signal rather than a quiet room: the input starts anew
        // after it. Silence the suppression made of noise is the room, as it sounds.
        let started = if !signal {
            self.started = 0;
            None
        } else {
            let frames = self.started;
            self.started = (frames + 1).min(SETTLE);
            Some(frames)
        };
        if started == Some(SETTLE) {
            self.track_floor(dbov, likely_speech);
        }
        let floor = self.floor_dbov();
        // Once unmuted, the floor from before tells speech; in a room not heard yet, the
        // denoiser settles first.
        let ready = started.is_some_and(|frames| frames == SETTLE || frames >= WARM_UP && floor.is_some());
        // Before the room was heard without speech, its floor is unknown and doesn't count.
        let above_floor = floor.is_none_or(|floor| dbov >= floor + ABOVE_FLOOR_DB);
        let voice = ready && likely_speech && dbov >= QUIETEST_VOICE_DBOV && above_floor;

        let (open_for, decision) = match (self.open_for, voice) {
            (None, true) => (Some(HOLD), Decision::Opens),
            (Some(_), true) => (Some(HOLD), Decision::Speaks),
            (Some(1), false) => (None, Decision::Closes),
            (Some(left), false) => (Some(left - 1), Decision::Speaks),
            (None, false) => (None, Decision::Quiet),
        };
        self.open_for = open_for;
        decision
    }

    /// Closes the gate for input that stopped, so the next frame starts anew, as after digital
    /// silence; whether it was open, with a talkspurt to end.
    pub fn end(&mut self) -> bool {
        self.started = 0;
        self.open_for.take().is_some()
    }

    fn track_floor(&mut self, dbov: f32, likely_speech: bool) {
        let stretch = (self.frames / FLOOR_STRETCH) as usize % FLOOR_STRETCHES;
        if self.frames.is_multiple_of(FLOOR_STRETCH) {
            self.heard = false;
        }
        if !likely_speech {
            let quietest = self.quietest[stretch].filter(|_| self.heard).map_or(dbov, |quietest| quietest.min(dbov));
            self.quietest[stretch] = Some(quietest);
            self.heard = true;
        }
        // Wrapping after 497 days starts a stretch early, which the floor doesn't mind.
        self.frames = self.frames.wrapping_add(1);
    }

    fn floor_dbov(&self) -> Option<f32> {
        self.quietest.iter().flatten().copied().reduce(f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: f32 = 0.9;
    const NOISE: f32 = 0.1;

    fn level(dbov: u8) -> Level {
        Level::new(dbov)
    }

    /// A gate past the denoiser's settling, in a room it hasn't heard without speech yet.
    fn settled() -> Gate {
        Gate { started: SETTLE, ..Gate::default() }
    }

    #[test]
    fn opens_on_speech_holds_through_a_pause_and_closes_after_it() {
        let mut gate = settled();
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Speaks);
        for _ in 1..HOLD {
            assert_eq!(gate.decide(NOISE, level(70), true), Decision::Speaks);
        }
        assert_eq!(gate.decide(NOISE, level(70), true), Decision::Closes);
        assert_eq!(gate.decide(NOISE, level(70), true), Decision::Quiet);
    }

    #[test]
    fn speech_in_the_pause_keeps_it_open() {
        let mut gate = settled();
        gate.decide(SPEECH, level(20), true);
        for _ in 0..HOLD - 1 {
            gate.decide(NOISE, level(70), true);
        }
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Speaks);
        for _ in 1..HOLD {
            assert_eq!(gate.decide(NOISE, level(70), true), Decision::Speaks);
        }
        assert_eq!(gate.decide(NOISE, level(70), true), Decision::Closes);
    }

    #[test]
    fn speech_right_after_it_closed_opens_it_anew() {
        let mut gate = settled();
        gate.decide(SPEECH, level(20), true);
        for _ in 0..HOLD {
            gate.decide(NOISE, level(70), true);
        }
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
    }

    #[test]
    fn ending_closes_it_and_the_input_settles_anew() {
        let mut gate = settled();
        assert!(!gate.end());
        gate = settled();
        gate.decide(SPEECH, level(20), true);
        assert!(gate.end());
        for _ in 0..SETTLE {
            assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Quiet);
        }
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
    }

    #[test]
    fn a_long_monologue_keeps_it_open() {
        let mut gate = settled();
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
        for _ in 0..3000 {
            assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Speaks);
        }
    }

    #[test]
    fn likely_speech_too_quiet_to_be_meant_stays_quiet() {
        let mut gate = settled();
        assert_eq!(gate.decide(SPEECH, level(60), true), Decision::Quiet);
    }

    #[test]
    fn loud_noise_that_isnt_speech_stays_quiet() {
        let mut gate = settled();
        assert_eq!(gate.decide(NOISE, level(10), true), Decision::Quiet);
    }

    #[test]
    fn speech_like_sound_near_a_risen_floor_stays_quiet_while_louder_speech_opens() {
        let mut gate = settled();
        // Game audio at -30 dBov sets the floor, ...
        for _ in 0..200 {
            assert_eq!(gate.decide(NOISE, level(30), true), Decision::Quiet);
        }
        // ... so a voice line in the game at that level, or a little louder, stays quiet.
        assert_eq!(gate.decide(SPEECH, level(30), true), Decision::Quiet);
        assert_eq!(gate.decide(SPEECH, level(25), true), Decision::Quiet);
        assert_eq!(gate.decide(SPEECH, level(15), true), Decision::Opens);
    }

    #[test]
    fn a_loud_room_sets_the_floor_at_once() {
        let mut gate = settled();
        assert_eq!(gate.decide(NOISE, level(30), true), Decision::Quiet);
        assert_eq!(gate.decide(SPEECH, level(28), true), Decision::Quiet);
    }

    #[test]
    fn a_floor_outlasts_sound_that_is_all_likely_speech() {
        let mut gate = settled();
        for _ in 0..200 {
            gate.decide(NOISE, level(35), true);
        }
        // Game music RNNoise takes for speech, a little above the floor, for longer than the floor spans.
        for _ in 0..1000 {
            assert_eq!(gate.decide(SPEECH, level(30), true), Decision::Quiet);
        }
    }

    #[test]
    fn in_a_room_not_heard_yet_nothing_opens_while_the_denoiser_settles() {
        let mut gate = Gate::default();
        for _ in 0..SETTLE {
            assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Quiet);
        }
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
    }

    #[test]
    fn in_a_room_heard_before_speech_right_after_digital_silence_opens_it_once_the_denoiser_warmed_up() {
        let mut gate = settled();
        gate.decide(NOISE, level(60), true);
        gate.decide(0.0, Level::SILENT, false);
        for _ in 0..WARM_UP {
            assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Quiet);
        }
        assert_eq!(gate.decide(SPEECH, level(20), true), Decision::Opens);
    }

    #[test]
    fn noise_the_denoiser_lets_through_while_it_settles_leaves_the_floor_as_it_was() {
        let mut gate = settled();
        for _ in 0..200 {
            gate.decide(NOISE, level(60), true);
        }
        gate.decide(0.0, Level::SILENT, false);
        for _ in 0..SETTLE {
            assert_eq!(gate.decide(NOISE, level(40), true), Decision::Quiet);
        }
        assert_eq!(gate.decide(SPEECH, level(45), true), Decision::Opens);
    }

    #[test]
    fn digital_silence_leaves_the_floor_to_the_room() {
        let mut gate = settled();
        for _ in 0..100 {
            gate.decide(0.0, Level::SILENT, false);
        }
        for _ in 0..SETTLE + 1 {
            gate.decide(NOISE, level(35), true);
        }
        assert_eq!(gate.decide(SPEECH, level(33), true), Decision::Quiet);
    }

    #[test]
    fn noise_the_suppression_silenced_is_the_room_not_input_that_stopped() {
        let mut gate = Gate::default();
        for _ in 0..SETTLE {
            assert_eq!(gate.decide(NOISE, Level::SILENT, true), Decision::Quiet);
        }
        assert_eq!(gate.decide(SPEECH, level(30), true), Decision::Opens);
    }

    #[test]
    fn the_floor_follows_the_room_within_a_second_and_a_half() {
        let mut gate = settled();
        for _ in 0..200 {
            gate.decide(NOISE, level(60), true);
        }
        // The game starts.
        for _ in 0..150 {
            gate.decide(NOISE, level(30), true);
        }
        assert_eq!(gate.decide(SPEECH, level(25), true), Decision::Quiet);
        // The game stops.
        for _ in 0..30 {
            gate.decide(NOISE, level(60), true);
        }
        assert_eq!(gate.decide(SPEECH, level(40), true), Decision::Opens);
    }
}
