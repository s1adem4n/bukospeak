//! Echo cancellation where the platform's audio doesn't cancel the speaker's echo itself, on the
//! desktop: what the speaker plays is the reference that WebRTC's AEC3, from `sonora`, takes out
//! of what the microphone hears.
//!
//! The speaker's frames cross to the capture through a ring, each with when it plays. For each
//! frame captured, the capture reads what played a little after it was captured, so AEC3 finds
//! the echo as late after its reference as the room makes it, whatever the devices' latencies.
//! The times come from the platform, which tells when a device plays or captured a callback's
//! first sample; unlike when the callbacks run, they don't depend on the scheduler. The two
//! devices run on clocks that drift apart by up to a few hundred parts per million, so each
//! device's times are fitted to a line, which smooths out their noise, follows the drift, and
//! starts anew where the device lost samples; the reference is resampled from the speaker's line
//! to the microphone's. Nothing here allocates after construction.

use std::time::{Duration, Instant};

use bukospeak_proto::audio::{FRAME_SAMPLES, SAMPLE_RATE};
use rtrb::{Consumer, Producer, RingBuffer};
use sonora::config::{EchoCanceller as Aec3, HighPassFilter, TransparentModeType};
use sonora::{AudioProcessing, Config, StreamConfig};

/// The frames the ring holds: 1 s, more than the time from mixing a frame to capturing its
/// echo takes through any two devices, Bluetooth's included.
const RING_FRAMES: usize = 100;

/// How long after a frame's capture what the reference holds for it plays: 5 ms, so that it
/// leads the echo, which AEC3 needs, even where a platform tells times a little off.
const LEAD_SECONDS: f64 = 0.005;

/// The span of the latest times told that make up a device's line: 30 s, which smooths times a
/// millisecond off to microseconds, and follows drift, which changes over minutes as the devices
/// warm up.
const HORIZON_SECONDS: f64 = 30.0;

/// How far a device's rate may be from the nominal, or from the rate before its callbacks
/// stopped, as a standard deviation: 300 ppm. A line holds to that rate while the times told
/// say less about it, as when they are few or noisy.
const PRIOR_DEVIATION: f64 = 300e-6;

/// The least noise the times told are taken to have: 20 µs, so a line doesn't trust a few
/// times as exact before it measured their noise.
const LEAST_NOISE_SECONDS: f64 = 20e-6;

/// A time told off a device's line by more than this, or than this many times the line's noise,
/// whichever is more, doesn't count; this many in a row, and the line starts anew from them:
/// the device lost or skipped samples.
const OFF_SECONDS: f64 = 0.001;
const OFF_NOISE: f64 = 6.0;
const OFF_IN_A_ROW: u32 = 3;

/// The times a line is told before it judges whether one is off.
const SETTLE_TOLD: u32 = 10;

/// Makes the ring the speaker's frames cross to the capture by.
pub(crate) fn reference() -> (Tap, Reference) {
    let (producer, consumer) = RingBuffer::new(RING_FRAMES);
    let epoch = Instant::now();
    (
        Tap { ring: producer, line: Line::new(), index: 0, epoch },
        Reference { ring: consumer, line: Line::new(), captured: 0, read: 0, epoch },
    )
}

/// A frame the speaker mixed, where it is in what the speaker played, and when it plays by the
/// speaker's line.
struct Played {
    samples: [f32; FRAME_SAMPLES],
    index: u64,
    /// In seconds since the epoch.
    at: f64,
    /// The speaker's samples per second.
    rate: f64,
}

/// When a device's samples pass: a line through the times told for them, weighted to the
/// latest, as an exponentially weighted linear regression. Its rate holds to a prior where the
/// times say little about it yet.
struct Line {
    told: u32,
    /// The mean of the samples' indices and of their times told, in seconds since the epoch;
    /// the co-moment of the two, and the indices' second moment.
    mean_index: f64,
    mean_seconds: f64,
    co_moment: f64,
    index_moment: f64,
    /// The sum of the times' squared weights, whose inverse is how many times the line is
    /// effectively made of.
    squared_weights: f64,
    last_seconds: f64,
    /// The rate the line holds to, in seconds per sample, and how precisely it is known, as the
    /// inverse of its variance; that fades as the times told before it would have.
    prior: f64,
    prior_precision: f64,
    /// The mean square of how far the times told are off the line.
    noise: f64,
    off: Option<Off>,
}

/// The latest of times told in a row off a line, and how many those are: the samples from it
/// on pass when it says, until the line finds it a fluke or starts anew from it.
#[derive(Clone, Copy)]
struct Off {
    index: f64,
    seconds: f64,
    in_a_row: u32,
}

impl Line {
    fn new() -> Self {
        let nominal = 1.0 / f64::from(SAMPLE_RATE);
        Self::holding_to(nominal, (nominal * PRIOR_DEVIATION).powi(-2), 0.0)
    }

    fn holding_to(prior: f64, prior_precision: f64, noise: f64) -> Self {
        Self {
            told: 0,
            mean_index: 0.0,
            mean_seconds: 0.0,
            co_moment: 0.0,
            index_moment: 0.0,
            squared_weights: 0.0,
            last_seconds: 0.0,
            prior,
            prior_precision,
            noise,
            off: None,
        }
    }

    /// Takes the time the platform told for the sample with this index.
    fn tell(&mut self, index: u64, seconds: f64) {
        let index = index as f64;
        let mut off = None;
        if self.told >= SETTLE_TOLD {
            let by = seconds - self.on_line(index);
            if by.abs() > OFF_SECONDS.max(OFF_NOISE * self.noise.sqrt()) {
                let in_a_row = self.off.map_or(0, |off| off.in_a_row) + 1;
                self.off = Some(Off { index, seconds, in_a_row });
                if in_a_row < OFF_IN_A_ROW {
                    return;
                }
                // The device lost or skipped samples: the line moved, but its rate didn't.
                let precision = self.fit_precision() + self.prior_precision;
                *self = Self::holding_to(self.seconds_per_sample(), precision, self.noise);
            } else {
                self.off = None;
                off = Some(by);
            }
        }
        let forget = if self.told == 0 { 0.0 } else { ((seconds - self.last_seconds) / HORIZON_SECONDS).clamp(0.0, 1.0) };
        self.told = self.told.saturating_add(1);
        let weight = (1.0 / f64::from(self.told)).max(forget);
        self.last_seconds = seconds;
        self.prior_precision *= 1.0 - forget;
        // Welford's updates, exponentially weighted.
        let (index_off, seconds_off) = (index - self.mean_index, seconds - self.mean_seconds);
        self.mean_index += weight * index_off;
        self.mean_seconds += weight * seconds_off;
        self.co_moment = (1.0 - weight) * (self.co_moment + weight * index_off * seconds_off);
        self.index_moment = (1.0 - weight) * (self.index_moment + weight * index_off * index_off);
        self.squared_weights = (1.0 - weight).powi(2) * self.squared_weights + weight * weight;
        if let Some(off) = off {
            self.noise += weight * (off * off - self.noise);
        }
    }

    /// Starts anew, for a device whose callbacks stopped and may come back from another
    /// device: holding to the rate so far, as loosely as to the nominal.
    fn restart(&mut self) {
        let prior = self.seconds_per_sample();
        *self = Self::holding_to(prior, (prior * PRIOR_DEVIATION).powi(-2), 0.0);
    }

    /// How precisely the times told give the rate, as the inverse of its variance: by how many
    /// they are, how far apart and how noisy.
    fn fit_precision(&self) -> f64 {
        if self.index_moment > 0.0 { self.index_moment / (self.squared_weights * self.noise.max(LEAST_NOISE_SECONDS.powi(2))) } else { 0.0 }
    }

    /// The fitted rate and the prior, each weighted by how precisely it is known.
    fn seconds_per_sample(&self) -> f64 {
        let precision = self.fit_precision();
        let fitted = if precision > 0.0 { self.co_moment / self.index_moment } else { self.prior };
        (fitted * precision + self.prior * self.prior_precision) / (precision + self.prior_precision)
    }

    /// When the sample with this index passes, in seconds since the epoch; None until told.
    fn seconds_at(&self, index: f64) -> Option<f64> {
        match self.off {
            Some(off) if index >= off.index => Some(off.seconds + (index - off.index) * self.seconds_per_sample()),
            _ => (self.told > 0).then(|| self.on_line(index)),
        }
    }

    fn on_line(&self, index: f64) -> f64 {
        self.mean_seconds + (index - self.mean_index) * self.seconds_per_sample()
    }

    fn rate(&self) -> f64 {
        1.0 / self.seconds_per_sample()
    }
}

/// How long `samples` take at 48 kHz.
pub(crate) fn duration_of(samples: usize) -> Duration {
    Duration::from_nanos(samples as u64 * 1_000_000_000 / u64::from(SAMPLE_RATE))
}

fn seconds_since(epoch: Instant, at: Instant) -> f64 {
    match at.checked_duration_since(epoch) {
        Some(after) => after.as_secs_f64(),
        None => -epoch.duration_since(at).as_secs_f64(),
    }
}

/// The speaker's end: hands over each frame it mixed, with when it plays.
pub(crate) struct Tap {
    ring: Producer<Played>,
    line: Line,
    /// The index of the next frame's first sample in what the speaker played.
    index: u64,
    epoch: Instant,
}

impl Tap {
    /// Hands over a frame the speaker mixed, whose first sample plays at `plays`.
    pub fn push(&mut self, frame: &[f32; FRAME_SAMPLES], plays: Instant) {
        self.line.tell(self.index, seconds_since(self.epoch, plays));
        let at = self.line.seconds_at(self.index as f64).expect("the line was just told");
        let played = Played { samples: *frame, index: self.index, at, rate: self.line.rate() };
        self.index += FRAME_SAMPLES as u64;
        // A full ring means the capture stopped reading; it reads by time, and finds the frames
        // missing silent.
        let _ = self.ring.push(played);
    }

    /// For when the platform's render callback stops calling.
    pub fn stopped(&mut self) {
        self.line.restart();
    }
}

/// The capture's end: what the speaker played as each frame was captured.
pub(crate) struct Reference {
    ring: Consumer<Played>,
    line: Line,
    /// The samples captured, and those of them read for, since the capture started.
    captured: u64,
    read: u64,
    epoch: Instant,
}

impl Reference {
    /// Counts `samples` the platform's capture callback hands over, the first of them captured
    /// at `at`.
    pub fn captured(&mut self, samples: usize, at: Instant) {
        self.line.tell(self.captured, seconds_since(self.epoch, at));
        self.captured += samples as u64;
    }

    /// For when the platform's capture callback stops calling: the samples after count anew.
    pub fn stopped(&mut self) {
        self.line.restart();
        (self.captured, self.read) = (0, 0);
    }

    /// Reads the reference for the next captured frame: what the speaker played as it was
    /// captured, and silence where the speaker played nothing.
    pub fn read(&mut self, frame: &mut [f32; FRAME_SAMPLES]) {
        frame.fill(0.0);
        let index = self.read;
        self.read += FRAME_SAMPLES as u64;
        let Some(captured_at) = self.line.seconds_at(index as f64) else {
            return;
        };
        let at = captured_at + LEAD_SECONDS;
        // The frames that played before the one playing at `at` are past.
        while let Ok(chunk) = self.ring.read_chunk(2) {
            let (first, second) = chunk.as_slices();
            if first.get(1).or(second.first()).is_some_and(|next| next.at <= at) {
                chunk.commit(1);
            } else {
                break;
            }
        }
        let chunk = self.ring.read_chunk(self.ring.slots()).expect("the ring holds the frames it has slots for");
        let (first, second) = chunk.as_slices();
        let Some(front) = first.first() else {
            return;
        };
        let start = front.index as f64 + (at - front.at) * front.rate;
        let step = front.rate / self.line.rate();
        // The speaker's sample with this index, if the ring holds it.
        let sample = |index: f64| -> f32 {
            if index < front.index as f64 {
                return 0.0;
            }
            let offset = (index as u64 - front.index) as usize;
            let frame = offset / FRAME_SAMPLES;
            let played = if frame < first.len() { first.get(frame) } else { second.get(frame - first.len()) };
            // A frame the ring dropped, a restart of the speaker's count, or one that plays
            // apart from the front's line, as after the speaker skipped, leaves a gap.
            played
                .filter(|played| {
                    let after = (frame * FRAME_SAMPLES) as u64;
                    played.index == front.index + after && (played.at - (front.at + after as f64 / front.rate)).abs() <= OFF_SECONDS
                })
                .map_or(0.0, |played| played.samples[offset % FRAME_SAMPLES])
        };
        for (n, out) in frame.iter_mut().enumerate() {
            let position = start + n as f64 * step;
            let whole = position.floor();
            let (before, after) = (sample(whole), sample(whole + 1.0));
            *out = before + (after - before) * (position - whole) as f32;
        }
    }
}

/// AEC3 with the high-pass filter it needs before it.
pub(crate) struct EchoCanceller {
    processing: AudioProcessing,
    input: [f32; FRAME_SAMPLES],
    /// Where the reference goes through, unchanged: `sonora` returns it processed.
    rendered: [f32; FRAME_SAMPLES],
}

impl EchoCanceller {
    pub fn new() -> Self {
        let config = Config {
            high_pass_filter: Some(HighPassFilter::default()),
            // Its hidden Markov model tells sooner than the default when there is no echo, as
            // on a headset, and then lets the voice through untouched.
            echo_canceller: Some(Aec3 { transparent_mode: TransparentModeType::Hmm, ..Aec3::default() }),
            ..Config::default()
        };
        let stream = StreamConfig::new(SAMPLE_RATE, 1);
        let processing = AudioProcessing::builder().config(config).capture_config(stream).render_config(stream).build();
        Self { processing, input: [0.0; FRAME_SAMPLES], rendered: [0.0; FRAME_SAMPLES] }
    }

    /// Takes the reference for the next captured frame, in −1.0..=1.0.
    pub fn render(&mut self, reference: &[f32; FRAME_SAMPLES]) {
        self.processing
            .process_render_f32(&[reference], &mut [&mut self.rendered])
            .expect("sonora takes 10 ms of 48 kHz mono, as configured");
    }

    /// Filters a captured frame, in −1.0..=1.0, and takes the echo out of it.
    pub fn capture(&mut self, frame: &mut [f32; FRAME_SAMPLES]) {
        self.input = *frame;
        self.processing.process_capture_f32(&[&self.input], &mut [frame]).expect("sonora takes 10 ms of 48 kHz mono, as configured");
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;
    use crate::signal::{noise, noise_from, voice};
    use crate::simulator::Room;

    const SECOND: usize = SAMPLE_RATE as usize;
    const SECOND_US: u64 = 1_000_000;

    /// Plays `far` in the room as the speaker does, while the microphone hears `near` and the
    /// echo, and hands `captured` each frame captured with its reference.
    fn run(
        room: &Room,
        far: &[f32],
        near: &[f32],
        duration_us: u64,
        mut captured: impl FnMut(&[f32; FRAME_SAMPLES], &[f32; FRAME_SAMPLES]),
    ) {
        let (mut tap, mut reference) = reference();
        let start = Instant::now();
        let mut far_frames = far.as_chunks::<FRAME_SAMPLES>().0.iter();
        let (mut mixed, mut played) = ([0.0; FRAME_SAMPLES], FRAME_SAMPLES);
        let (mut frame, mut filled) = ([0.0; FRAME_SAMPLES], 0);
        let mut reference_frame = [0.0; FRAME_SAMPLES];
        room.run(
            near,
            duration_us,
            |plays, samples| {
                // As the speaker does: mixes a frame when it played the one before.
                for (offset, sample) in samples.iter_mut().enumerate() {
                    if played == FRAME_SAMPLES {
                        mixed = far_frames.next().copied().unwrap_or([0.0; FRAME_SAMPLES]);
                        tap.push(&mixed, start + plays + duration_of(offset));
                        played = 0;
                    }
                    *sample = mixed[played];
                    played += 1;
                }
            },
            |at, samples| {
                reference.captured(samples.len(), start + at);
                for &sample in samples {
                    frame[filled] = sample;
                    filled += 1;
                    if filled == FRAME_SAMPLES {
                        filled = 0;
                        reference.read(&mut reference_frame);
                        captured(&frame, &reference_frame);
                    }
                }
            },
        );
    }

    /// The speaker plays a ramp, each sample its index, and the microphone hears it as it plays:
    /// by how many samples each frame's reference is off what played as it was captured, past
    /// the lead, at its first sample captured.
    fn misaligned(room: &Room, seconds: usize) -> Vec<f32> {
        let ramp: Vec<f32> = (0..seconds * SECOND).map(|n| n as f32).collect();
        let lead = (LEAD_SECONDS * f64::from(SAMPLE_RATE)) as f32;
        let mut off = Vec::new();
        run(&Room { echo: &[(0, 1.0)], ..*room }, &ramp, &[], seconds as u64 * SECOND_US, |captured, reference| {
            off.push(if captured[0] == 0.0 || reference[0] == 0.0 { f32::NAN } else { reference[0] - captured[0] - lead });
        });
        off
    }

    /// Asserts that once the lines settled, after `settle` seconds, each frame's reference is
    /// aligned within `samples`, but for `but` frames.
    fn assert_aligned(room: &Room, settle: usize, samples: f32, but: usize) {
        let off = misaligned(room, 60);
        let misaligned: Vec<(usize, f32)> =
            off.iter().copied().enumerate().skip(settle * 100).filter(|&(_, off)| off.is_nan() || off.abs() > samples).collect();
        assert!(misaligned.len() <= but, "{} frames misaligned, the first {:?}", misaligned.len(), &misaligned[..misaligned.len().min(10)]);
    }

    #[test]
    fn the_reference_is_what_played_as_the_frame_was_captured() {
        // The room's times, 200 µs off, leave the lines a sample or two off in their first
        // seconds, and less after.
        assert_aligned(&Room::default(), 3, 2.0, 0);
        // Bluetooth, on clocks that drift apart.
        let bluetooth = Room {
            output_drift_ppm: 250.0,
            input_drift_ppm: -250.0,
            output_latency_us: 200_000,
            input_latency_us: 40_000,
            ..Room::default()
        };
        assert_aligned(&bluetooth, 3, 2.0, 0);
    }

    #[test]
    fn the_reference_stays_aligned_through_noisy_times_and_late_callbacks() {
        // Times 2 ms off leave the lines off by a few samples.
        let noisy = Room {
            output_drift_ppm: -250.0,
            input_drift_ppm: 250.0,
            timestamp_jitter_us: 2_000,
            callback_jitter_us: 8_000,
            ..Room::default()
        };
        assert_aligned(&noisy, 10, 8.0, 0);
    }

    #[test]
    fn the_reference_aligns_anew_after_a_device_lost_samples() {
        let glitching = Room {
            output_drift_ppm: 250.0,
            input_drift_ppm: -250.0,
            // A buffer lost, and 300 ms; a buffer of silence played, and 600 ms.
            input_lost: &[(1_000, 1), (3_000, 33)],
            output_stalls: &[(1_000, 1), (2_000, 28)],
            ..Room::default()
        };
        // Each time, the new line is a sample or two off for half a second.
        assert_aligned(&glitching, 2, 1.0, 4 * 60);
    }

    #[test]
    fn a_line_follows_a_drifting_device_through_noise_and_starts_anew_after_a_gap() {
        let mut line = Line::new();
        let rate = 48_000.0 * (1.0 + 300e-6);
        let mut random = 0x2545_f491_u32;
        let mut noise = || {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            (f64::from(random % 2_001) - 1_000.0) * 1e-6
        };
        let mut told = |line: &mut Line, from: u64, to: u64, gap: f64| {
            for callback in from..to {
                let index = callback * 480;
                line.tell(index, index as f64 / rate + gap + noise());
            }
        };
        told(&mut line, 0, 6_000, 0.0);
        // Times a millisecond off leave the line off by about 0.4 ppm and 15 µs at its head, as
        // standard deviations.
        let off_ppm = (line.rate() / rate - 1.0).abs() * 1e6;
        assert!(off_ppm <= 2.0, "{off_ppm:.1} ppm off after a minute");
        let index = 6_000.0 * 480.0;
        let off_us = (line.seconds_at(index).unwrap() - index / rate).abs() * 1e6;
        assert!(off_us <= 60.0, "{off_us:.0} µs off after a minute");

        // The device skips 10 ms.
        told(&mut line, 6_000, 6_010, 0.01);
        let off_us = (line.seconds_at(index).unwrap() - (index / rate + 0.01)).abs() * 1e6;
        assert!(off_us <= 1_000.0, "{off_us:.0} µs off after the gap");
        assert!((line.rate() / rate - 1.0).abs() <= 3e-6, "the rate is held");
    }

    /// What the microphone captured and what is left after echo cancellation.
    struct Cancelled {
        captured: Vec<f32>,
        cancelled: Vec<f32>,
    }

    /// Plays `far` in the room while the microphone hears `near` and the echo, and cancels it.
    fn cancel(room: &Room, far: &[f32], near: &[f32], duration_us: u64) -> Cancelled {
        let mut canceller = EchoCanceller::new();
        let (mut captured, mut cancelled) = (Vec::new(), Vec::new());
        run(room, far, near, duration_us, |frame, reference| {
            canceller.render(reference);
            captured.extend_from_slice(frame);
            let mut frame = *frame;
            canceller.capture(&mut frame);
            cancelled.extend_from_slice(&frame);
        });
        Cancelled { captured, cancelled }
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|sample| sample * sample).sum()
    }

    /// How much quieter `after` is than `before`, in dB, over the seconds from `from` to `to`,
    /// or to the end of what was captured.
    fn quieter_db(before: &[f32], after: &[f32], from: usize, to: usize) -> f32 {
        let seconds = from * SECOND..(to * SECOND).min(before.len());
        10.0 * (energy(&before[seconds.clone()]) / energy(&after[seconds])).log10()
    }

    /// Someone talking on the far end, as the mix plays them, over a noisy line.
    fn far(seconds: usize) -> Vec<f32> {
        voice(seconds * SECOND, 0.3).iter().zip(noise(seconds * SECOND, 0.03)).map(|(voice, noise)| voice + noise).collect()
    }

    /// Someone talking on the near end: noise in syllables, most of it below 4 kHz, as speech.
    /// A second synthetic voice would correlate with the far end's, as no two people do, and
    /// AEC3 would take one for the other's echo.
    fn near(seconds: usize, peak: f32) -> Vec<f32> {
        let noise = noise_from(0x9e37_79b9, seconds * SECOND, peak);
        // Averaged over 8 samples, which leaves an eighth of its power above 4 kHz.
        let averaged = noise.windows(8).map(|window| window.iter().sum::<f32>() / 8.0_f32.sqrt());
        averaged.enumerate().map(|(n, sample)| sample * (0.5 - 0.5 * (TAU * 3.0 * n as f32 / SECOND as f32).cos())).collect()
    }

    /// Asserts that the echo is cancelled by `db` once AEC3 converged, in each of the last 40 s
    /// of a minute but `but` of them.
    fn assert_cancelled(room: &Room, db: f32, but: usize) {
        let cancelled = cancel(room, &far(60), &[], 60 * SECOND_US);
        let short: Vec<(usize, f32)> = (20..60)
            .map(|second| (second, quieter_db(&cancelled.captured, &cancelled.cancelled, second, second + 1)))
            .filter(|&(_, suppression)| suppression.is_nan() || suppression < db)
            .collect();
        assert!(short.len() <= but, "cancelled by less than {db} dB in seconds {short:.1?}");
    }

    #[test]
    fn the_echo_is_cancelled() {
        assert_cancelled(&Room::default(), 40.0, 0);
    }

    #[test]
    fn the_echo_is_cancelled_while_the_clocks_drift_apart() {
        assert_cancelled(&Room { output_drift_ppm: 300.0, input_drift_ppm: -200.0, ..Room::default() }, 40.0, 0);
        // A buffer each per 10 ms, as on Windows, close to the echo.
        assert_cancelled(
            &Room {
                output_drift_ppm: 250.0,
                input_drift_ppm: -250.0,
                output_buffer: 480,
                input_buffer: 480,
                output_latency_us: 12_000,
                input_latency_us: 4_000,
                echo: &[(1_000, 0.5)],
                ..Room::default()
            },
            40.0,
            0,
        );
    }

    #[test]
    fn the_echo_is_cancelled_through_bluetooth() {
        assert_cancelled(&Room { output_latency_us: 200_000, input_latency_us: 40_000, ..Room::default() }, 40.0, 0);
    }

    #[test]
    fn the_echo_is_cancelled_through_noisy_times() {
        // Times 2 ms off wander the reference by a few samples, which AEC3 follows a little
        // behind.
        assert_cancelled(&Room { timestamp_jitter_us: 2_000, callback_jitter_us: 8_000, ..Room::default() }, 25.0, 0);
    }

    #[test]
    fn the_echo_is_cancelled_again_within_a_second_after_lost_samples() {
        // A buffer lost, and 300 ms; a buffer of silence played, and 600 ms.
        let glitching = Room { input_lost: &[(3_000, 1), (5_000, 33)], output_stalls: &[(1_500, 1), (2_200, 28)], ..Room::default() };
        assert_cancelled(&glitching, 40.0, 4);
    }

    #[test]
    fn without_echo_the_voice_passes() {
        let room = Room { echo: &[], ..Room::default() };
        let cancelled = cancel(&room, &far(60), &near(60, 0.2), 60 * SECOND_US);
        // AEC3 tells within seconds that there is no echo, and a little later that for sure.
        for from in (5..55).step_by(5) {
            let loss = quieter_db(&cancelled.captured, &cancelled.cancelled, from, from + 5);
            assert!(loss <= 6.0, "the voice lost {loss:.1} dB from {from} s");
        }
        let loss = quieter_db(&cancelled.captured, &cancelled.cancelled, 35, 60);
        assert!(loss.abs() <= 1.0, "the voice lost {loss:.1} dB");
    }

    #[test]
    fn the_voice_passes_while_both_ends_talk() {
        let near: Vec<f32> = [vec![0.0; 30 * SECOND], near(20, 0.2)].concat();
        let with_echo = cancel(&Room::default(), &far(50), &near, 50 * SECOND_US);
        let without = cancel(&Room { echo: &[], ..Room::default() }, &far(50), &near, 50 * SECOND_US);
        let loss = quieter_db(&without.cancelled, &with_echo.cancelled, 30, 50);
        assert!(loss <= 6.0, "the voice lost {loss:.1} dB to the echo's cancellation");
    }

    #[test]
    fn reading_and_cancelling_dont_allocate() {
        let (mut tap, mut reference) = reference();
        let mut canceller = EchoCanceller::new();
        let start = Instant::now();
        let far = far(2);
        let frames = far.as_chunks::<FRAME_SAMPLES>().0;
        let mut frame = [0.0; FRAME_SAMPLES];
        assert_no_alloc::assert_no_alloc(|| {
            for (index, played) in frames.iter().enumerate() {
                let now = start + Duration::from_millis(10 * index as u64);
                tap.push(played, now + Duration::from_millis(30));
                reference.captured(FRAME_SAMPLES, now - Duration::from_millis(10));
                reference.read(&mut frame);
                canceller.render(&frame);
                canceller.capture(&mut frame);
            }
        });
    }
}
