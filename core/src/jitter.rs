//! A sender's frames, from the order and pace the network delivers them to one frame per 10 ms.
//!
//! The render callback owns one buffer per sender, so nothing here allocates or blocks after
//! construction. The buffer waits for its target delay before a talkspurt starts and keeps
//! close to it while playing: holding too much, it drops the quietest of the next frames; too
//! little, it inserts concealment before a quiet frame, or wherever it runs dry. Its target
//! follows the arrival jitter of the last two seconds.

use bukospeak_proto::audio::{FRAME_DURATION, Level, MAX_OPUS_LEN};

/// Frames the buffer can hold: well beyond the longest target.
const SLOTS: usize = 64;

/// The fewest frames a talkspurt starts with: one to play and one to recover a loss from by FEC.
const MIN_TARGET: u32 = 2;
const MAX_TARGET: u32 = 20;

/// Frames beyond the target the buffer tolerates before it drops one.
const HEADROOM: u32 = 2;

/// Frames ahead among which a drop looks for the quietest.
const DROP_WINDOW: u32 = 5;

/// Frames played between two adjustments, so they stay inaudible.
const ADJUST_EVERY: u32 = 10;

/// A frame at least this far below full scale is quiet enough to insert concealment before.
const QUIET: Level = Level::new(45);

/// Concealed frames in a row after which the talkspurt counts as over: its last frame was lost.
const GIVE_UP_AFTER: u32 = 10;

/// A gap in the frames ahead longer than this is a new talkspurt or a long outage: the buffer
/// starts over rather than concealing through it.
const MAX_GAP: u32 = 5;

const FRAME_US: u64 = FRAME_DURATION.as_micros() as u64;

// A packet's length is stored as a u16.
const _: () = assert!(MAX_OPUS_LEN <= u16::MAX as usize);

/// A frame as the buffer holds it: fixed size, so storing it never allocates.
#[derive(Clone, Copy, Debug)]
pub struct Packet {
    pub index: u32,
    pub last: bool,
    pub level: Level,
    /// When it arrived, in microseconds on the receiver's monotonic clock.
    pub arrival_us: u64,
    len: u16,
    opus: [u8; MAX_OPUS_LEN],
}

impl Packet {
    /// None if the Opus packet is longer than [`MAX_OPUS_LEN`].
    pub fn new(index: u32, last: bool, level: Level, arrival_us: u64, opus: &[u8]) -> Option<Self> {
        let mut bytes = [0; MAX_OPUS_LEN];
        bytes.get_mut(..opus.len())?.copy_from_slice(opus);
        Some(Self { index, last, level, arrival_us, len: opus.len() as u16, opus: bytes })
    }

    pub fn opus(&self) -> &[u8] {
        &self.opus[..usize::from(self.len)]
    }
}

/// What to play for the next 10 ms.
#[derive(Debug, PartialEq, Eq)]
pub enum Playout<'a> {
    /// The sender is quiet.
    Silent,
    /// Decode this Opus packet.
    Frame(&'a [u8]),
    /// The frame is lost and the next one is here: decode the next frame's Opus packet for the
    /// copy of the lost one it carries (in-band FEC).
    Fec(&'a [u8]),
    /// The frame is lost, late or inserted: conceal it with Opus' loss concealment.
    Conceal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub played: u64,
    pub recovered: u64,
    pub concealed: u64,
    pub dropped: u64,
    pub inserted: u64,
    /// Frames that arrived after their turn or beyond the window held, and were thrown away.
    pub discarded: u64,
}

pub struct JitterBuffer {
    /// Frames by index modulo [`SLOTS`]. A slot keeps its frame until another overwrites it;
    /// whether the frame is still to play follows from the state.
    slots: [Option<Packet>; SLOTS],
    state: State,
    since_adjustment: u32,
    jitter: Jitter,
    stats: Stats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Between talkspurts. Frames up to `over` belong to the last one and don't start another;
    /// it is None until the first frame arrives.
    Idle { over: Option<u32>, waited: u32 },
    /// `next` plays next; `concealed` frames in a row were concealed before it.
    Playing { next: u32, concealed: u32 },
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            state: State::Idle { over: None, waited: 0 },
            since_adjustment: 0,
            jitter: Jitter::default(),
            stats: Stats::default(),
        }
    }
}

impl JitterBuffer {
    pub fn insert(&mut self, packet: Packet) {
        // A late frame still tells how much the delay spreads.
        self.jitter.observe(packet.index, packet.arrival_us);
        if let State::Idle { over, waited } = self.state
            && self.oldest_pending().is_none()
        {
            // The first frame of a talkspurt anchors the window of pending frames, leaving room
            // for earlier ones that arrive after it, but not for the last talkspurt's.
            let anchor = packet.index.wrapping_sub(SLOTS as u32 / 2);
            let over = over.filter(|&over| distance(over, anchor) < 0).unwrap_or(anchor);
            self.state = State::Idle { over: Some(over), waited };
        }
        if let State::Idle { over: Some(over), waited } = self.state
            && distance(over, packet.index) > SLOTS as i64
        {
            // A frame beyond the window moves it on: the frames it pushes out are too old to wait for.
            self.state = State::Idle { over: Some(packet.index.wrapping_sub(SLOTS as u32)), waited };
        }
        if !self.is_pending(packet.index) {
            self.stats.discarded += 1;
            return;
        }
        self.slots[slot(packet.index)] = Some(packet);
    }

    /// The next 10 ms; the render callback asks once per frame it plays.
    pub fn pop(&mut self) -> Playout<'_> {
        self.since_adjustment = self.since_adjustment.saturating_add(1);
        let (next, concealed) = match self.state {
            State::Playing { next, concealed } => (next, concealed),
            State::Idle { over, waited } => match self.start(waited) {
                Some(next) => (next, 0),
                None => {
                    // Waiting counts from the talkspurt's first frame on.
                    let waited = if self.oldest_pending().is_some() { waited + 1 } else { 0 };
                    self.state = State::Idle { over, waited };
                    return Playout::Silent;
                }
            },
        };

        if let Some(packet) = self.pending(next) {
            let level = packet.level;
            if self.should_drop(next, level) {
                self.since_adjustment = 0;
                self.stats.dropped += 1;
                self.state = State::Playing { next: next.wrapping_add(1), concealed: 0 };
                // The frame after it takes its 10 ms; no second adjustment follows so soon.
                return self.pop();
            }
            if self.should_insert(next, level) {
                self.since_adjustment = 0;
                self.stats.inserted += 1;
                self.state = State::Playing { next, concealed };
                return Playout::Conceal;
            }
            return self.play(next);
        }

        let following = next.wrapping_add(1);
        if self.pending(following).is_some() {
            self.stats.recovered += 1;
            self.state = State::Playing { next: following, concealed: 0 };
            return Playout::Fec(self.slots[slot(following)].as_ref().expect("the following frame is held").opus());
        }

        match self.oldest_pending().map(|oldest| distance(next, oldest)) {
            // Nothing ahead: the frame is late, or the talkspurt's last one was lost. Waiting
            // for it inserts a frame.
            None => {
                self.stats.inserted += 1;
                self.conceal(next, concealed)
            }
            Some(gap) if gap > i64::from(MAX_GAP) => {
                self.state = State::Idle { over: Some(next.wrapping_sub(1)), waited: 0 };
                Playout::Silent
            }
            Some(_) => {
                self.stats.concealed += 1;
                self.conceal(following, concealed)
            }
        }
    }

    /// Whether a talkspurt is playing, which is when its sender counts as speaking.
    pub fn is_playing(&self) -> bool {
        matches!(self.state, State::Playing { .. })
    }

    /// The frames the buffer aims to hold.
    pub fn target(&self) -> u32 {
        self.jitter.target()
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Forgets every frame, as after the audio engine was interrupted.
    pub fn flush(&mut self) {
        self.slots = [None; SLOTS];
        self.state = State::Idle { over: None, waited: 0 };
    }

    /// The first frame of a talkspurt once enough of it is held: its target delay, its last
    /// frame, or a wait as long as the target, whichever comes first.
    fn start(&mut self, waited: u32) -> Option<u32> {
        let (oldest, newest) = (self.oldest_pending()?, self.newest_pending()?);
        let held = distance(oldest, newest) as u32 + 1;
        let ending = self.pending_frames().any(|packet| packet.last);
        if held >= self.target() || ending || waited >= self.target() {
            self.since_adjustment = 0;
            Some(oldest)
        } else {
            None
        }
    }

    fn play(&mut self, index: u32) -> Playout<'_> {
        self.stats.played += 1;
        let last = self.pending(index).expect("the frame is held").last;
        self.state =
            if last { State::Idle { over: Some(index), waited: 0 } } else { State::Playing { next: index.wrapping_add(1), concealed: 0 } };
        Playout::Frame(self.slots[slot(index)].as_ref().expect("the frame is held").opus())
    }

    fn conceal(&mut self, next: u32, concealed: u32) -> Playout<'static> {
        if concealed + 1 >= GIVE_UP_AFTER {
            self.state = State::Idle { over: Some(next.wrapping_sub(1)), waited: 0 };
            return Playout::Silent;
        }
        self.state = State::Playing { next, concealed: concealed + 1 };
        Playout::Conceal
    }

    /// Frames held from `next` on, counting the gaps between them.
    fn ahead(&self, next: u32) -> u32 {
        self.newest_pending().map_or(0, |newest| distance(next, newest) as u32 + 1)
    }

    fn should_drop(&self, next: u32, level: Level) -> bool {
        if self.ahead(next) <= self.target() + HEADROOM || self.since_adjustment < ADJUST_EVERY {
            return false;
        }
        // The frame drops if none in the window is quieter: a rising level drops at once, a
        // falling one waits for its quietest frame to come up.
        (1..DROP_WINDOW).filter_map(|offset| self.pending(next.wrapping_add(offset))).all(|packet| packet.level <= level)
    }

    fn should_insert(&self, next: u32, level: Level) -> bool {
        self.ahead(next) + 1 < self.target() && level >= QUIET && self.since_adjustment >= ADJUST_EVERY
    }

    /// Whether a frame with this index is still to play: it lies in the window of [`SLOTS`]
    /// frames after the last one played, so pending frames never share a slot.
    fn is_pending(&self, index: u32) -> bool {
        match self.state {
            State::Playing { next, .. } => (0..SLOTS as i64).contains(&distance(next, index)),
            State::Idle { over: Some(over), .. } => (1..=SLOTS as i64).contains(&distance(over, index)),
            State::Idle { over: None, .. } => false,
        }
    }

    fn pending(&self, index: u32) -> Option<&Packet> {
        self.slots[slot(index)].as_ref().filter(|packet| packet.index == index && self.is_pending(index))
    }

    fn pending_frames(&self) -> impl Iterator<Item = &Packet> {
        self.slots.iter().flatten().filter(|packet| self.is_pending(packet.index))
    }

    fn oldest_pending(&self) -> Option<u32> {
        let reference = self.reference()?;
        self.pending_frames().map(|packet| packet.index).min_by_key(|&index| distance(reference, index))
    }

    fn newest_pending(&self) -> Option<u32> {
        let reference = self.reference()?;
        self.pending_frames().map(|packet| packet.index).max_by_key(|&index| distance(reference, index))
    }

    /// The point the window of pending frames starts at, which orders them on the wrapping timeline.
    fn reference(&self) -> Option<u32> {
        match self.state {
            State::Playing { next, .. } => Some(next),
            State::Idle { over, .. } => over,
        }
    }
}

/// How far `to` lies after `from` on a timeline that wraps around.
fn distance(from: u32, to: u32) -> i64 {
    i64::from(to.wrapping_sub(from) as i32)
}

fn slot(index: u32) -> usize {
    index as usize % SLOTS
}

/// The spread of the arrival delay over the last two seconds, in [`Jitter::BUCKETS`] buckets of
/// 200 ms.
///
/// A frame's relative delay is its arrival time minus its place on the sender's timeline. The
/// clocks' offset cancels out of the spread, and their drift barely shows in two seconds.
#[derive(Default)]
struct Jitter {
    buckets: [Option<Bucket>; Jitter::BUCKETS],
    newest: u64,
}

#[derive(Clone, Copy)]
struct Bucket {
    id: u64,
    min: i64,
    max: i64,
}

impl Jitter {
    const BUCKETS: usize = 10;
    const BUCKET_US: u64 = 200_000;

    fn observe(&mut self, index: u32, arrival_us: u64) {
        let delay = arrival_us as i64 - i64::from(index) * FRAME_US as i64;
        let id = arrival_us / Self::BUCKET_US;
        let bucket = &mut self.buckets[id as usize % Self::BUCKETS];
        match bucket {
            Some(bucket) if bucket.id == id => {
                bucket.min = bucket.min.min(delay);
                bucket.max = bucket.max.max(delay);
            }
            _ => *bucket = Some(Bucket { id, min: delay, max: delay }),
        }
        self.newest = id.max(self.newest);
    }

    fn target(&self) -> u32 {
        let recent = self.buckets.iter().flatten().filter(|bucket| bucket.id + Self::BUCKETS as u64 > self.newest);
        let (min, max) = recent.fold((i64::MAX, i64::MIN), |(min, max), bucket| (min.min(bucket.min), max.max(bucket.max)));
        if min > max {
            return MIN_TARGET;
        }
        let spread = (max - min) as u64;
        (spread.div_ceil(FRAME_US) as u32 + 1).clamp(MIN_TARGET, MAX_TARGET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulator::{Played, Simulation, heard};

    const SECOND_US: u64 = 1_000_000;

    /// How long each played frame waited after it was sent, in milliseconds.
    fn delays_ms(played: &[Played], drift_ppm: f64) -> Vec<f64> {
        played
            .iter()
            .enumerate()
            .filter_map(|(tick, played)| match *played {
                Played::Frame(index) => {
                    let sent_us = f64::from(index) * FRAME_US as f64 / (1.0 + drift_ppm / 1e6);
                    Some((tick as f64 * FRAME_US as f64 - sent_us) / 1000.0)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_clean_network_plays_every_frame_in_order() {
        let (played, buffer) = Simulation::default().run(0..300, 4 * SECOND_US);
        assert_eq!(heard(&played), (0..300).collect::<Vec<_>>());
        let talkspurt = played.iter().skip_while(|played| **played == Played::Silent).take(300);
        assert!(talkspurt.into_iter().all(|played| matches!(played, Played::Frame(_))));
        assert_eq!(buffer.stats(), Stats { played: 300, ..Stats::default() });
        assert!(!buffer.is_playing());
    }

    #[test]
    fn a_single_loss_is_recovered_by_fec() {
        let simulation = Simulation { lost: |index| index % 20 == 10, ..Simulation::default() };
        let (played, buffer) = simulation.run(0..300, 4 * SECOND_US);
        assert_eq!(heard(&played), (0..300).collect::<Vec<_>>());
        assert_eq!((buffer.stats().recovered, buffer.stats().concealed), (15, 0));
    }

    #[test]
    fn a_burst_loss_is_concealed_and_playback_continues() {
        let simulation = Simulation { lost: |index| (100..103).contains(&index), ..Simulation::default() };
        let (played, buffer) = simulation.run(0..300, 4 * SECOND_US);
        assert_eq!(heard(&played), (0..100).chain(102..300).collect::<Vec<_>>());
        assert_eq!((buffer.stats().concealed, buffer.stats().recovered), (2, 1));
    }

    #[test]
    fn jitter_raises_the_target_and_reordered_frames_play_in_order() {
        let simulation = Simulation { jitter_us: 40_000, ..Simulation::default() };
        let (played, buffer) = simulation.run(0..1000, 12 * SECOND_US);
        let heard = heard(&played);
        assert!(heard.is_sorted_by(|a, b| a < b), "out of order: {heard:?}");
        assert!(heard.len() >= 980, "only {} of 1000 frames heard", heard.len());
        assert_eq!(buffer.target(), 5);
    }

    #[test]
    fn a_fast_sender_s_drift_is_absorbed_by_dropping_frames_even_without_quiet_ones() {
        let drift_ppm = 1000.0;
        let simulation = Simulation { drift_ppm, ..Simulation::default() };
        let (played, buffer) = simulation.run(0..12_000, 125 * SECOND_US);
        // 1000 ppm over 120 s are 12 frames: without drops the delay would have grown by 120 ms.
        let delays = delays_ms(&played, drift_ppm);
        let latest = delays[delays.len() - 100..].iter().copied().fold(f64::MIN, f64::max);
        assert!(latest <= 20.0 + f64::from((MIN_TARGET + HEADROOM + 1) * 10), "delay grew to {latest} ms");
        assert!(buffer.stats().dropped >= 8, "{:?}", buffer.stats());
    }

    #[test]
    fn a_slow_sender_s_drift_is_absorbed_by_inserting_frames() {
        let simulation = Simulation { drift_ppm: -1000.0, ..Simulation::default() };
        let (played, buffer) = simulation.run(0..12_000, 125 * SECOND_US);
        assert_eq!(heard(&played), (0..12_000).collect::<Vec<_>>());
        assert!(buffer.stats().inserted >= 8, "{:?}", buffer.stats());
        assert_eq!(buffer.stats().concealed, 0);
    }

    #[test]
    fn drops_take_the_quietest_frame_in_reach() {
        let level = |index| Level::new(if index % 4 == 0 { 90 } else { 20 });
        let simulation = Simulation { drift_ppm: 1000.0, level, ..Simulation::default() };
        let (played, _) = simulation.run(0..12_000, 125 * SECOND_US);
        let heard = heard(&played);
        let dropped: Vec<u32> = (0..12_000).filter(|index| heard.binary_search(index).is_err()).collect();
        assert!(!dropped.is_empty());
        assert!(dropped.iter().all(|index| index % 4 == 0), "loud frames dropped: {dropped:?}");
    }

    #[test]
    fn the_last_frame_ends_the_talkspurt() {
        let (played, buffer) = Simulation::default().run(0..50, SECOND_US);
        let end = played.iter().position(|played| *played == Played::Frame(49)).unwrap();
        assert!(played[end + 1..].iter().all(|played| *played == Played::Silent));
        assert!(!buffer.is_playing());
    }

    #[test]
    fn a_lost_last_frame_ends_the_talkspurt_after_a_timeout() {
        let simulation = Simulation { lost: |index| index == 49, ..Simulation::default() };
        let (played, buffer) = simulation.run(0..50, SECOND_US);
        let end = played.iter().position(|played| *played == Played::Frame(48)).unwrap();
        let concealed = played[end + 1..].iter().take_while(|played| **played == Played::Conceal).count();
        assert_eq!(concealed as u32, GIVE_UP_AFTER - 1);
        assert!(played[end + 1 + concealed..].iter().all(|played| *played == Played::Silent));
        assert!(!buffer.is_playing());
    }

    #[test]
    fn talkspurts_after_a_pause_and_after_a_lost_last_frame_start_fresh() {
        let simulation = Simulation { lost: |index| index == 199, ..Simulation::default() };
        let (played, _) = simulation.run((0..100).chain(103..200).chain(300..400), 5 * SECOND_US);
        assert_eq!(heard(&played), (0..100).chain(103..199).chain(300..400).collect::<Vec<_>>());
    }

    fn packet(index: u32, last: bool, arrival_us: u64) -> Packet {
        Packet::new(index, last, Level::new(20), arrival_us, &index.to_be_bytes()).unwrap()
    }

    #[test]
    fn stragglers_of_a_finished_talkspurt_are_discarded() {
        let mut buffer = JitterBuffer::default();
        buffer.insert(packet(0, false, 0));
        buffer.insert(packet(2, true, 20_000));
        let played: Vec<Played> = (0..3).map(|_| buffer.pop().into()).collect();
        assert_eq!(played, [Played::Frame(0), Played::Fec(1), Played::Frame(2)]);
        buffer.insert(packet(1, false, 30_000));
        assert_eq!(buffer.pop(), Playout::Silent);
        assert_eq!(buffer.stats().discarded, 1);
    }

    #[test]
    fn a_burst_longer_than_the_window_keeps_its_newest_frames() {
        let mut buffer = JitterBuffer::default();
        for index in 0..100 {
            buffer.insert(packet(index, index == 99, u64::from(index) * FRAME_US));
        }
        assert_eq!(Played::from(buffer.pop()), Played::Frame(100 - SLOTS as u32));
        assert_eq!(buffer.stats().discarded, 0);
    }

    #[test]
    fn frames_play_in_order_across_the_index_wrapping_around() {
        let mut buffer = JitterBuffer::default();
        for (offset, index) in [u32::MAX - 1, u32::MAX, 0, 1].into_iter().enumerate() {
            buffer.insert(packet(index, index == 1, offset as u64 * FRAME_US));
        }
        let played: Vec<Played> = (0..5).map(|_| buffer.pop().into()).collect();
        assert_eq!(played, [Played::Frame(u32::MAX - 1), Played::Frame(u32::MAX), Played::Frame(0), Played::Frame(1), Played::Silent]);
    }

    #[test]
    fn flush_forgets_every_frame() {
        let mut buffer = JitterBuffer::default();
        buffer.insert(packet(0, false, 0));
        buffer.insert(packet(1, false, FRAME_US));
        buffer.flush();
        assert_eq!(buffer.pop(), Playout::Silent);
        buffer.insert(packet(1, false, 2 * FRAME_US));
        buffer.insert(packet(2, false, 3 * FRAME_US));
        assert_eq!(Played::from(buffer.pop()), Played::Frame(1));
    }
}
