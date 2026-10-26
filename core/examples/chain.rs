//! Runs a recording through the capture and plays it as a listener hears it:
//! `chain IN.wav OUT.wav [--platform] [--raw | --rnnoise]`, 48 kHz mono in and out.
//!
//! The capture conditions the signal as on the desktop unless `--platform` says the platform's
//! audio did, as on the iPhone. It suppresses noise with DeepFilterNet3 where the feature
//! `deepfilter` builds it in, else or with `--rnnoise` with RNNoise; `--raw` sends the signal
//! without noise suppression. It prints how much later the listener hears the recording.

use std::error::Error;
use std::time::Duration;

use bukospeak_core::Conditioning;
use bukospeak_core::harness::{self, Options, Suppression};
use bukospeak_core::proto::audio::SAMPLE_RATE;

/// The excerpt the delay is measured on, in seconds.
const EXCERPT: (usize, usize) = (5, 10);

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, flags @ ..] = args.as_slice() else {
        return Err("usage: chain IN.wav OUT.wav [--platform] [--raw | --rnnoise]".into());
    };
    let flag = |name: &str| flags.iter().any(|flag| flag == name);
    let conditioning = if flag("--platform") { Conditioning::Platform } else { Conditioning::Engine };
    let suppression = match (flag("--raw"), flag("--rnnoise")) {
        (true, _) => Suppression::Off,
        #[cfg(feature = "deepfilter")]
        (false, false) => Suppression::DeepFilter,
        _ => Suppression::RNNoise,
    };
    let options = Options { conditioning, suppression };

    let mut reader = hound::WavReader::open(input)?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != SAMPLE_RATE {
        return Err(format!("{input} has {} channels at {} Hz, not one at {SAMPLE_RATE} Hz", spec.channels, spec.sample_rate).into());
    }
    let recording: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|sample| sample.map(|sample| sample as f32 / scale)).collect::<Result<_, _>>()?
        }
    };

    let played = harness::play(&recording, options)?;
    let spec = hound::WavSpec { channels: 1, sample_rate: SAMPLE_RATE, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(output, spec)?;
    played.iter().try_for_each(|sample| writer.write_sample(*sample))?;
    writer.finalize()?;

    let rate = SAMPLE_RATE as usize;
    if let (Some(captured), Some(heard)) = (recording.get(EXCERPT.0 * rate..EXCERPT.1 * rate), played.get(EXCERPT.0 * rate..)) {
        println!("delay {:?}", harness::delay(captured, heard, Duration::from_millis(100)));
    }
    Ok(())
}
