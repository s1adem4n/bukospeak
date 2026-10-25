//! Echo cancellation where the platform's audio doesn't cancel the speaker's echo itself, on the
//! desktop: what the speaker plays is the reference that WebRTC's AEC3, from `sonora`, takes out
//! of what the microphone hears.
//!
//! The speaker's mix crosses to the capture callback through a ring of samples. The two
//! callbacks run on the clocks of two devices, which drift apart by up to a few hundred parts
//! per million. Each end measures its device's rate against the monotonic clock, and the
//! capture reads the ring at the speaker's rate by resampling it a little, so the echo's delay
//! stays put and AEC3's filter stays converged. The ring's level only aligns the reader, once
//! it starts; AEC3 finds the delay itself. Nothing here allocates after construction.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bukospeak_proto::audio::{FRAME_SAMPLES, SAMPLE_RATE};
use rtrb::{Consumer, Producer, RingBuffer};
use sonora::config::{EchoCanceller as Aec3, HighPassFilter, TransparentModeType};
use sonora::{AudioProcessing, Config, StreamConfig};

/// What the ring holds at most: 200 ms of the speaker's mix.
const RING_SAMPLES: usize = SAMPLE_RATE as usize / 5;

/// What the ring holds when the reader starts: 40 ms, more than the target and a frame, and
/// than it lacks between two of the speaker's buffers.
const START_SAMPLES: usize = SAMPLE_RATE as usize * 4 / 100;

/// What the ring should hold at its lowest, once a frame is read: 15 ms, so the reference stays
/// there before its echo through two frames captured without a buffer played between them,
/// which the second the reader aligns in may not show, and the drift until the devices' rates
/// are told; and ahead of it by little more than the devices' latency.
const TARGET_SAMPLES: usize = SAMPLE_RATE as usize * 15 / 1000;

/// More than this, 100 ms, and the reader skips back to the start level and aligns anew: the
/// capture stopped while the speaker played on.
const MOST_SAMPLES: usize = SAMPLE_RATE as usize / 10;

/// The frames in which the reader, once started, finds the ring's lowest level to align by: 1 s.
const ALIGN_FRAMES: u32 = 100;

/// How long a device runs before its rate is told: 5 s, after which callbacks a few
/// milliseconds late leave it a few dozen ppm off, and less the longer it runs.
const SETTLE_SECONDS: f64 = 5.0;

/// Devices drift apart by less: 1000 ppm.
const MOST_CORRECTION: f64 = 1e-3;

/// Makes the ring the speaker's mix crosses to the capture by.
pub(crate) fn reference() -> (Tap, Reference) {
    let (producer, consumer) = RingBuffer::new(RING_SAMPLES);
    let output_rate = Arc::new(AtomicU64::new(0));
    (
        Tap { ring: producer, clock: Clock::default(), rate: Arc::clone(&output_rate) },
        Reference {
            ring: consumer,
            clock: Clock::default(),
            output_rate,
            reading: false,
            aligned: false,
            before: 0.0,
            after: 0.0,
            fraction: 0.0,
            step: 1.0,
            lowest: usize::MAX,
            frames: 0,
        },
    )
}

/// A device's sample rate by the monotonic clock: the slope of the samples it asked for or
/// handed over against when its callbacks ran, fitted to all of them since it started, so their
/// lateness averages out.
#[derive(Default)]
struct Clock {
    started: Option<Instant>,
    /// The samples of the callbacks before.
    samples: u64,
    callbacks: u64,
    /// The callbacks' mean time since the start and mean samples before them, and their
    /// co-moments, kept as Welford does, which stays exact over hours.
    mean_seconds: f64,
    mean_samples: f64,
    seconds_by_samples: f64,
    seconds_squared: f64,
}

impl Clock {
    /// Counts the samples of a callback at `now`, and tells the rate once the device ran long
    /// enough.
    fn tick(&mut self, now: Instant, samples: usize) -> Option<f64> {
        let started = *self.started.get_or_insert(now);
        let seconds = now.saturating_duration_since(started).as_secs_f64();
        let before = self.samples as f64;
        self.samples += samples as u64;
        self.callbacks += 1;
        let count = self.callbacks as f64;
        let seconds_off = seconds - self.mean_seconds;
        self.mean_seconds += seconds_off / count;
        self.mean_samples += (before - self.mean_samples) / count;
        self.seconds_by_samples += seconds_off * (before - self.mean_samples);
        self.seconds_squared += seconds_off * (seconds - self.mean_seconds);
        (seconds >= SETTLE_SECONDS).then(|| self.seconds_by_samples / self.seconds_squared)
    }

    /// Starts measuring anew, for a device whose callbacks stopped and may come back from
    /// another device.
    fn restart(&mut self) {
        *self = Self::default();
    }
}

/// The speaker's end: hands over each frame it mixed, and the rate the speaker plays at.
pub(crate) struct Tap {
    ring: Producer<f32>,
    clock: Clock,
    /// The output device's rate in samples per second, as `f64` bits; 0 until told.
    rate: Arc<AtomicU64>,
}

impl Tap {
    /// Counts the samples the platform's render callback asks for at `now`.
    pub fn played(&mut self, now: Instant, samples: usize) {
        if let Some(rate) = self.clock.tick(now, samples) {
            self.rate.store(rate.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn push(&mut self, frame: &[f32; FRAME_SAMPLES]) {
        // A full ring means the capture stopped reading; it skips ahead once it reads again.
        let _ = self.ring.push_partial_slice(frame);
    }

    /// For when the platform's render callback stops calling; the rate told stays until the
    /// next is.
    pub fn stopped(&mut self) {
        self.clock.restart();
    }
}

/// The capture's end: what the speaker played, read at the capture's pace.
///
/// It starts once the ring holds the start level, and reads for a second; then it skips ahead
/// so that the ring's lowest level is the target, which aligns it. It starts anew after the
/// ring ran dry, and aligns anew after it filled up.
pub(crate) struct Reference {
    ring: Consumer<f32>,
    clock: Clock,
    output_rate: Arc<AtomicU64>,
    /// Whether the reader started since the ring last ran dry; until then the reference is
    /// silence.
    reading: bool,
    /// Whether the reader skipped to the target since it started.
    aligned: bool,
    /// The two samples the reader is between, and how far it is from the first to the second.
    before: f32,
    after: f32,
    fraction: f64,
    /// The speaker's samples the reader takes per sample captured: the ratio of the devices'
    /// rates.
    step: f64,
    /// The ring's lowest level, and the frames read, since the reader started.
    lowest: usize,
    frames: u32,
}

impl Reference {
    /// Counts the samples the platform's capture callback hands over at `now`.
    pub fn captured(&mut self, now: Instant, samples: usize) {
        let Some(input_rate) = self.clock.tick(now, samples) else {
            return;
        };
        let output_rate = f64::from_bits(self.output_rate.load(Ordering::Relaxed));
        if output_rate > 0.0 {
            self.step = 1.0 + (output_rate / input_rate - 1.0).clamp(-MOST_CORRECTION, MOST_CORRECTION);
        }
    }

    /// For when the platform's capture callback stops calling; the ratio stays until the next
    /// is told.
    pub fn stopped(&mut self) {
        self.clock.restart();
    }

    /// Reads the reference for a captured frame.
    pub fn read(&mut self, frame: &mut [f32; FRAME_SAMPLES]) {
        let level = self.ring.slots();
        if !self.reading && level >= START_SAMPLES {
            self.reading = true;
            self.realign();
        }
        if level > MOST_SAMPLES {
            self.skip(level - START_SAMPLES);
            self.realign();
        }
        if !self.reading {
            frame.fill(0.0);
            return;
        }
        for (index, sample) in frame.iter_mut().enumerate() {
            self.fraction += self.step;
            while self.fraction >= 1.0 {
                let Ok(next) = self.ring.pop() else {
                    // Ran dry: the speaker stopped. The reader waits for the start level again,
                    // and AEC3 finds the new delay.
                    self.reading = false;
                    self.fraction = 0.0;
                    frame[index..].fill(0.0);
                    return;
                };
                (self.before, self.after) = (self.after, next);
                self.fraction -= 1.0;
            }
            *sample = self.before + (self.after - self.before) * self.fraction as f32;
        }
        if self.aligned {
            return;
        }
        // What is left once the frame is read: what the reference is ahead by.
        self.lowest = self.lowest.min(self.ring.slots());
        self.frames += 1;
        if self.frames == ALIGN_FRAMES {
            self.skip(self.lowest - self.lowest.min(TARGET_SAMPLES));
            self.aligned = true;
        }
    }

    fn realign(&mut self) {
        self.aligned = false;
        self.lowest = usize::MAX;
        self.frames = 0;
    }

    /// Drops samples the ring holds; it holds at least as many.
    fn skip(&mut self, samples: usize) {
        self.ring.read_chunk(samples).expect("the ring holds the samples skipped").commit_all();
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
    use std::time::Duration;

    use super::*;
    use crate::signal::{noise, voice, voice_at};
    use crate::simulator::Room;

    const SECOND: usize = SAMPLE_RATE as usize;
    const SECOND_US: u64 = 1_000_000;

    /// What the microphone captured and what is left after echo cancellation; and the ring's
    /// end the capture read by.
    struct Cancelled {
        captured: Vec<f32>,
        cancelled: Vec<f32>,
        reference: Reference,
        /// How often the reader ran dry.
        ran_dry: usize,
    }

    /// Plays `far` in the room while the microphone hears `near` and the echo, and cancels it.
    fn cancel(room: &Room, far: &[f32], near: &[f32], duration_us: u64) -> Cancelled {
        let (mut tap, mut reference) = reference();
        let mut canceller = EchoCanceller::new();
        let epoch = Instant::now();
        let mut far_frames = far.as_chunks::<FRAME_SAMPLES>().0.iter();
        let (mut mixed, mut played) = ([0.0; FRAME_SAMPLES], FRAME_SAMPLES);
        let (mut frame, mut filled) = ([0.0; FRAME_SAMPLES], 0);
        let mut reference_frame = [0.0; FRAME_SAMPLES];
        let (mut captured, mut cancelled, mut ran_dry) = (Vec::new(), Vec::new(), 0);
        room.run(
            near,
            duration_us,
            |at, samples| {
                tap.played(epoch + at, samples.len());
                // As the speaker does: mixes a frame when it played the one before.
                for sample in samples {
                    if played == FRAME_SAMPLES {
                        mixed = far_frames.next().copied().unwrap_or([0.0; FRAME_SAMPLES]);
                        tap.push(&mixed);
                        played = 0;
                    }
                    *sample = mixed[played];
                    played += 1;
                }
            },
            |at, samples| {
                reference.captured(epoch + at, samples.len());
                for &sample in samples {
                    frame[filled] = sample;
                    filled += 1;
                    if filled == FRAME_SAMPLES {
                        filled = 0;
                        let reading = reference.reading;
                        reference.read(&mut reference_frame);
                        ran_dry += usize::from(reading && !reference.reading);
                        canceller.render(&reference_frame);
                        captured.extend_from_slice(&frame);
                        canceller.capture(&mut frame);
                        cancelled.extend_from_slice(&frame);
                    }
                }
            },
        );
        Cancelled { captured, cancelled, reference, ran_dry }
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

    #[test]
    fn the_echo_is_cancelled() {
        let cancelled = cancel(&Room::default(), &far(30), &[], 30 * SECOND_US);
        let suppression = quieter_db(&cancelled.captured, &cancelled.cancelled, 10, 30);
        assert!(suppression >= 40.0, "{suppression:.1} dB");
        assert_eq!(cancelled.ran_dry, 0);
    }

    #[test]
    fn the_echo_is_cancelled_while_the_clocks_drift_apart() {
        let rooms = [
            Room { output_drift_ppm: 300.0, input_drift_ppm: -200.0, ..Room::default() },
            Room { output_drift_ppm: -250.0, input_drift_ppm: 250.0, ..Room::default() },
            // A buffer each per 10 ms, as on Windows, whose order turns as the clocks drift.
            Room { output_drift_ppm: -250.0, input_drift_ppm: 250.0, output_buffer: 480, input_buffer: 480, ..Room::default() },
            Room {
                output_drift_ppm: 250.0,
                input_drift_ppm: -250.0,
                output_buffer: 480,
                input_buffer: 480,
                output_latency_us: 12_000,
                input_latency_us: 4_000,
                echo: &[(1_000, 0.5)],
                ..Room::default()
            },
        ];
        for room in rooms {
            let cancelled = cancel(&room, &far(60), &[], 60 * SECOND_US);
            let drift = (room.output_drift_ppm, room.input_drift_ppm);
            let ratio = (1.0 + room.output_drift_ppm / 1e6) / (1.0 + room.input_drift_ppm / 1e6);
            let off_ppm = (cancelled.reference.step - ratio).abs() * 1e6;
            assert!(off_ppm <= 5.0, "the reader's pace is {off_ppm:.1} ppm off at {drift:?} ppm");
            assert_eq!(cancelled.ran_dry, 0, "at {drift:?} ppm");
            let suppression = quieter_db(&cancelled.captured, &cancelled.cancelled, 20, 60);
            assert!(suppression >= 40.0, "{suppression:.1} dB at {drift:?} ppm");
        }
    }

    #[test]
    fn without_echo_the_voice_passes() {
        let room = Room { echo: &[], ..Room::default() };
        let near = voice_at(210.0, 60 * SECOND, 0.2);
        let cancelled = cancel(&room, &far(60), &near, 60 * SECOND_US);
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
        // Noise in syllables: two synthetic voices of the same pulse train correlate, as no two
        // people do, and AEC3 would take one for the other's echo.
        let talk = noise(20 * SECOND, 0.2)
            .into_iter()
            .enumerate()
            .map(|(n, sample)| sample * (0.5 - 0.5 * (TAU * 3.0 * n as f32 / SECOND as f32).cos()));
        let near: Vec<f32> = [vec![0.0; 30 * SECOND], talk.collect()].concat();
        let with_echo = cancel(&Room::default(), &far(50), &near, 50 * SECOND_US);
        let without = cancel(&Room { echo: &[], ..Room::default() }, &far(50), &near, 50 * SECOND_US);
        let loss = quieter_db(&without.cancelled, &with_echo.cancelled, 30, 50);
        assert!(loss <= 6.0, "the voice lost {loss:.1} dB to the echo's cancellation");
    }

    /// Hands over and reads `count` frames, one after the other, as the speaker and the capture
    /// run; the frames handed over are numbered from `first`.
    fn frames(tap: &mut Tap, reference: &mut Reference, first: &mut f32, count: usize, speaker: bool, capture: bool) -> Vec<f32> {
        let mut read = Vec::new();
        for _ in 0..count {
            if speaker {
                tap.push(&[*first; FRAME_SAMPLES]);
                *first += 1.0;
            }
            if capture {
                let mut frame = [0.0; FRAME_SAMPLES];
                reference.read(&mut frame);
                read.extend_from_slice(&frame);
            }
        }
        read
    }

    #[test]
    fn the_reader_starts_anew_after_the_capture_or_the_speaker_stopped() {
        let (mut tap, mut reference) = reference();
        let mut next = 1.0;
        // The speaker starts 50 ms ahead.
        frames(&mut tap, &mut reference, &mut next, 5, true, false);
        frames(&mut tap, &mut reference, &mut next, 150, true, true);
        assert!(reference.reading && reference.aligned);

        // The capture stops for 300 ms while the speaker plays on: the ring fills up.
        frames(&mut tap, &mut reference, &mut next, 30, true, false);
        let read = frames(&mut tap, &mut reference, &mut next, 150, true, true);
        assert!(reference.reading && reference.aligned);
        assert!(read.iter().all(|&sample| sample != 0.0), "the reader skipped ahead without a gap");

        // The speaker stops: the reader runs dry and waits for the start level again.
        frames(&mut tap, &mut reference, &mut next, 10, false, true);
        assert!(!reference.reading);
        frames(&mut tap, &mut reference, &mut next, 5, true, false);
        frames(&mut tap, &mut reference, &mut next, 150, true, true);
        assert!(reference.reading && reference.aligned);
    }

    #[test]
    fn a_rate_is_told_despite_late_callbacks() {
        let mut clock = Clock::default();
        let start = Instant::now();
        let rate = 48_000.0 * (1.0 + 300e-6);
        let mut random = 0x2545_f491_u32;
        let mut told = None;
        for callback in 0..1_000u32 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let late = Duration::from_micros(u64::from(random % 3_000));
            let on_time = Duration::from_secs_f64(f64::from(callback * 480) / rate);
            told = clock.tick(start + on_time + late, 480);
            assert_eq!(told.is_some(), on_time + late >= Duration::from_secs_f64(SETTLE_SECONDS));
        }
        let off_ppm = (told.unwrap() / rate - 1.0).abs() * 1e6;
        assert!(off_ppm <= 30.0, "{off_ppm:.1} ppm off after 10 s");
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
                tap.played(now, FRAME_SAMPLES);
                tap.push(played);
                reference.captured(now, FRAME_SAMPLES);
                reference.read(&mut frame);
                canceller.render(&frame);
                canceller.capture(&mut frame);
            }
        });
    }
}
