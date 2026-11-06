//! Measures an effect on this machine's card: `measure ROOT IN.wav OUT.wav [--effect NAME]
//! [--architecture NAME] [--log]`, 48 kHz mono in, the effect's output out; Studio Voice unless
//! `--effect` names the other, on the card found unless `--architecture` names one.
//!
//! It runs the recording through the effect twice, frames back to back and then each at the
//! time it would arrive from a microphone, as the GPU clocks down between them in a call. It
//! prints how long loading took, how long the frames took against their budget, and how much
//! later the voice comes out.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bukospeak_maxine::{Architecture, Effect, FRAME, Kind, Runtime, SAMPLE_RATE, Severity};

const USAGE: &str = "usage: measure ROOT IN.wav OUT.wav [--effect NAME] [--architecture NAME] [--log]";
/// Silence after the recording, so what the effect delays comes out too.
const TAIL: Duration = Duration::from_millis(500);
/// The longest delay looked for.
const MOST_DELAY: Duration = Duration::from_millis(300);

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, input, output, flags @ ..] = args.as_slice() else {
        return Err(USAGE.into());
    };
    let mut kind = Kind::StudioVoiceLowLatency;
    let mut architecture = None;
    let mut log = false;
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--effect" => kind = flags.next().ok_or(USAGE)?.parse()?,
            "--architecture" => architecture = Some(flags.next().ok_or(USAGE)?.parse()?),
            "--log" => log = true,
            _ => return Err(USAGE.into()),
        }
    }
    let architecture: Architecture = match architecture {
        Some(architecture) => architecture,
        None => bukospeak_maxine::detect()?.ok_or("no RTX card found")?,
    };

    let started = Instant::now();
    let runtime = Arc::new(Runtime::load(Path::new(root))?);
    println!("runtime loaded in {:.2?}", started.elapsed());
    if log {
        runtime.log(Severity::Info)?;
    }
    let started = Instant::now();
    let mut effect = Effect::new(&runtime, kind, architecture)?;
    println!("{} loaded for {} in {:.2?}", kind.name(), architecture.name(), started.elapsed());

    let recording = read(input)?;
    let period = Duration::from_secs_f64(FRAME as f64 / f64::from(SAMPLE_RATE));
    let (times, _) = run(&mut effect, &recording, None)?;
    report("back to back", &times, period);
    effect.reset()?;
    let (times, processed) = run(&mut effect, &recording, Some(period))?;
    report("paced", &times, period);

    let spec = hound::WavSpec { channels: 1, sample_rate: SAMPLE_RATE, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(output, spec)?;
    processed.iter().try_for_each(|sample| writer.write_sample(*sample))?;
    writer.finalize()?;
    let delay = delay(&recording, &processed);
    println!("delay {delay:.1?}, {} frames as the effect tells", kind.delay());
    Ok(())
}

fn read(path: &str) -> Result<Vec<f32>, Box<dyn Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != SAMPLE_RATE {
        return Err(format!("{path} has {} channels at {} Hz, not one at {SAMPLE_RATE} Hz", spec.channels, spec.sample_rate).into());
    }
    Ok(match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|sample| sample.map(|sample| sample as f32 / scale)).collect::<Result<_, _>>()?
        }
    })
}

/// Runs the recording and the tail after it through the effect, each frame at its time from
/// the start if paced by `period`, and returns how long each frame took and what came out.
fn run(effect: &mut Effect, recording: &[f32], period: Option<Duration>) -> Result<(Vec<Duration>, Vec<f32>), Box<dyn Error>> {
    let tail = (TAIL.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize;
    let mut signal = recording.to_vec();
    signal.resize((recording.len() + tail).next_multiple_of(FRAME), 0.0);
    let mut times = Vec::new();
    let mut processed = Vec::with_capacity(signal.len());
    let mut frame_out = [0.0; FRAME];
    let start = Instant::now();
    for (index, frame_in) in signal.as_chunks::<FRAME>().0.iter().enumerate() {
        if let Some(period) = period {
            let due = start + period * index as u32;
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        let started = Instant::now();
        effect.run(frame_in, &mut frame_out)?;
        times.push(started.elapsed());
        processed.extend_from_slice(&frame_out);
    }
    Ok((times, processed))
}

/// Prints the first frame's time apart, as the GPU warms up on it, then the others'.
fn report(name: &str, times: &[Duration], period: Duration) {
    let [first, rest @ ..] = times else { return };
    let mut rest = rest.to_vec();
    rest.sort();
    let at = |quantile: f64| rest[((rest.len() - 1) as f64 * quantile) as usize];
    let over = rest.iter().filter(|time| **time > period).count();
    let mean = rest.iter().sum::<Duration>() / rest.len() as u32;
    println!(
        "{name}: first {first:.2?}, then mean {mean:.2?}, median {:.2?}, p99 {:.2?}, p99.9 {:.2?}, max {:.2?}; {over} of {} over the {period:.0?} budget",
        at(0.5),
        at(0.99),
        at(0.999),
        at(1.0),
        rest.len()
    );
}

/// How much later `processed` carries the voice of `recording`: the lag at which their
/// loudness correlates best. Studio Voice makes the voice anew, so its waveform needn't follow
/// the recording's, but its loudness does.
fn delay(recording: &[f32], processed: &[f32]) -> Duration {
    // Loudness at 2 kHz, smoothed over a millisecond: lags in half milliseconds.
    const STEP: usize = 24;
    let loudness = |signal: &[f32]| -> Vec<f64> {
        let mut level = 0.0;
        let smoothed: Vec<f64> = signal
            .iter()
            .map(|sample| {
                level += (f64::from(sample.abs()) - level) / 48.0;
                level
            })
            .step_by(STEP)
            .collect();
        let mean = smoothed.iter().sum::<f64>() / smoothed.len().max(1) as f64;
        smoothed.into_iter().map(|level| level - mean).collect()
    };
    let (recording, processed) = (loudness(recording), loudness(processed));
    let most = (MOST_DELAY.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize / STEP;
    let correlation = |lag: usize| -> f64 { recording.iter().zip(processed.iter().skip(lag)).map(|(a, b)| a * b).sum() };
    let lag = (0..=most).max_by(|a, b| correlation(*a).total_cmp(&correlation(*b))).unwrap_or(0);
    Duration::from_secs_f64((lag * STEP) as f64 / f64::from(SAMPLE_RATE))
}
