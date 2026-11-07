use std::error::Error;
use std::time::Duration;

use bukospeak_core::harness::{self, Options};
use bukospeak_core::proto::audio::SAMPLE_RATE;
use bukospeak_core::{Conditioning, Suppression};

const EXCERPT_SECONDS: (usize, usize) = (5, 10);

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
    if let (Some(captured), Some(heard)) =
        (recording.get(EXCERPT_SECONDS.0 * rate..EXCERPT_SECONDS.1 * rate), played.get(EXCERPT_SECONDS.0 * rate..))
    {
        println!("delay {:?}", harness::delay(captured, heard, Duration::from_millis(100)));
    }
    Ok(())
}
