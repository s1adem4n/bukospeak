//! The trimmed inference against the fork's over a signal that takes every stage, for the
//! default settings and with the post filter and an attenuation limit. `fork_default.f32` and
//! `fork_filtered.f32` hold the fork's output for the two. The fork ran on tract 0.21 and this
//! crate runs on 0.23, whose kernels round differently, so the two agree within float rounding,
//! not bit for bit.

use df::{DfParams, DfTract, RuntimeParams, DEEPFILTERNET3};

const FORK_DEFAULT: &[u8] = include_bytes!("fork_default.f32");
const FORK_FILTERED: &[u8] = include_bytes!("fork_filtered.f32");
const HOP: usize = 480;
const FRAMES: usize = 200;

/// Hops 0..50 a loud voice-like harmonic signal alone, 50..100 noise alone, 100..200 the voice
/// in noise; the voice's pitch glides and its loudness rises and falls like syllables.
fn signal() -> Vec<f32> {
    let mut state: u32 = 0x1234_5678;
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(HOP * FRAMES);
    for n in 0..HOP * FRAMES {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = (state as f64 / u32::MAX as f64) * 2.0 - 1.0;
        let t = n as f64 / 48_000.0;
        let f0 = 150.0 + 40.0 * (2.0 * std::f64::consts::PI * 0.7 * t).sin();
        phase += 2.0 * std::f64::consts::PI * f0 / 48_000.0;
        let mut voice = 0.0;
        for k in 1..=30 {
            let f = k as f64 * f0;
            let formants = (-((f - 700.0) / 300.0).powi(2)).exp()
                + 0.6 * (-((f - 1800.0) / 400.0).powi(2)).exp()
                + 0.3 * (-((f - 2800.0) / 500.0).powi(2)).exp();
            voice += formants * (k as f64 * phase).sin() / k as f64;
        }
        voice *= (2.0 * std::f64::consts::PI * 3.0 * t).sin().max(0.0);
        let x = match n / HOP {
            0..50 => 0.5 * voice + 0.0005 * noise,
            50..100 => 0.02 * noise,
            _ => 0.3 * voice + 0.03 * noise,
        };
        out.push(x as f32);
    }
    out
}

fn run(model: &mut DfTract, input: &[f32]) -> Vec<f32> {
    let mut output = Vec::with_capacity(input.len());
    let mut enhanced = [0.0; HOP];
    for frame in input.as_chunks::<HOP>().0 {
        model.process(frame, &mut enhanced).unwrap();
        output.extend_from_slice(&enhanced);
    }
    output
}

fn model(runtime: &RuntimeParams) -> DfTract {
    DfTract::new(DfParams::from_bytes(DEEPFILTERNET3).unwrap(), runtime).unwrap()
}

/// Panics unless `ours` is the fork's output up to float rounding: no sample off by more than
/// a millionth, and the difference's energy at least 120 dB below the output's.
fn assert_matches(ours: &[f32], fork: &[u8]) {
    let fork: Vec<f32> = fork.as_chunks::<4>().0.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect();
    assert_eq!(ours.len(), fork.len());
    let energy = |samples: &mut dyn Iterator<Item = f32>| samples.map(|x| f64::from(x).powi(2)).sum::<f64>();
    let most = ours.iter().zip(&fork).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    let error = energy(&mut ours.iter().zip(&fork).map(|(a, b)| a - b));
    let snr = 10.0 * (energy(&mut fork.iter().copied()) / error).log10();
    assert!(most < 1e-6 && snr > 120.0, "off by up to {most}, {snr} dB below the output");
}

#[test]
fn the_output_is_the_fork_s() {
    let input = signal();
    assert_matches(&run(&mut model(&RuntimeParams::default()), &input), FORK_DEFAULT);
    let filtered = RuntimeParams { post_filter_beta: 0.02, atten_lim_db: 30., ..RuntimeParams::default() };
    assert_matches(&run(&mut model(&filtered), &input), FORK_FILTERED);
}

#[test]
fn a_reset_model_is_a_new_one() {
    let input = signal();
    let mut used = model(&RuntimeParams::default());
    run(&mut used, &input[..120 * HOP]);
    used.reset().unwrap();
    assert_eq!(run(&mut used, &input), run(&mut model(&RuntimeParams::default()), &input));
}

#[test]
fn the_output_comes_a_window_and_the_lookahead_later() {
    let model = model(&RuntimeParams::default());
    assert_eq!((model.hop_size(), model.delay()), (HOP, 1440));
}
