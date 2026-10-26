//! DeepFilterNet's real-time inference, trimmed to one channel and a model read from memory:
//! [`DfTract`] enhances 48 kHz audio one hop at a time. The signal processing around the model,
//! the STFT, the features it takes and the gains and deep filter it returns, is here; the model
//! runs on tract, in [`tract`].

use std::sync::Arc;

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub mod tract;

pub use tract::{DfParams, DfTract, Error, RuntimeParams};

/// The DeepFilterNet3 model's archive, for [`DfParams::from_bytes`].
pub const DEEPFILTERNET3: &[u8] = include_bytes!("../models/DeepFilterNet3.tar.gz");

pub type Complex32 = num_complex::Complex32;

const MEAN_NORM_INIT: [f32; 2] = [-60., -90.];
const UNIT_NORM_INIT: [f32; 2] = [0.001, 0.0001];

fn freq2erb(freq_hz: f32) -> f32 {
    9.265 * (freq_hz / (24.7 * 9.265)).ln_1p()
}

fn erb2freq(n_erb: f32) -> f32 {
    24.7 * 9.265 * ((n_erb / 9.265).exp() - 1.)
}

/// The STFT, the features' normalization states and the ERB bands of one channel.
#[derive(Clone)]
pub(crate) struct DFState {
    frame_size: usize,  // hop_size
    window_size: usize, // Same as fft_size
    freq_size: usize,   // fft_size / 2 + 1
    fft_forward: Arc<dyn RealToComplex<f32>>,
    fft_inverse: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    wnorm: f32,
    erb: Vec<usize>, // frequencies bandwidth (in bands) per ERB band
    analysis_mem: Vec<f32>,
    analysis_scratch: Vec<Complex32>,
    analysis_buf: Vec<f32>, // Persistent FFT input; avoids a per-hop allocation
    synthesis_mem: Vec<f32>,
    synthesis_scratch: Vec<Complex32>,
    synthesis_buf: Vec<f32>, // Persistent IFFT output; avoids a per-hop allocation
    mean_norm_state: Vec<f32>,
    unit_norm_state: Vec<f32>,
}

fn erb_fb(sr: usize, fft_size: usize, nb_bands: usize, min_nb_freqs: usize) -> Vec<usize> {
    // Init ERB filter bank
    let nyq_freq = sr / 2;
    let freq_width = sr as f32 / fft_size as f32;
    let erb_low: f32 = freq2erb(0.);
    let erb_high: f32 = freq2erb(nyq_freq as f32);
    let mut erb = vec![0; nb_bands];
    let step = (erb_high - erb_low) / nb_bands as f32;
    let min_nb_freqs = min_nb_freqs as i32; // Minimum number of frequency bands per erb band
    let mut prev_freq = 0; // Last frequency band of the previous erb band
    let mut freq_over = 0; // Number of frequency bands that are already stored in previous erb bands
    for i in 1..nb_bands + 1 {
        let f = erb2freq(erb_low + i as f32 * step);
        let fb = (f / freq_width).round() as usize;
        let mut nb_freqs = fb as i32 - prev_freq as i32 - freq_over;
        if nb_freqs < min_nb_freqs {
            // Not enough freq bins in current bark bin
            freq_over = min_nb_freqs - nb_freqs; // keep track of number of enforced bins
            nb_freqs = min_nb_freqs; // enforce min_nb_freqs
        } else {
            freq_over = 0
        }
        erb[i - 1] = nb_freqs as usize;
        prev_freq = fb;
    }
    erb[nb_bands - 1] += 1; // since we have WINDOW_SIZE/2+1 frequency bins
    let too_large = erb.iter().sum::<usize>() - (fft_size / 2 + 1);
    if too_large > 0 {
        erb[nb_bands - 1] -= too_large;
    }
    debug_assert!(erb.iter().sum::<usize>() == fft_size / 2 + 1);
    erb
}

impl DFState {
    pub(crate) fn new(sr: usize, fft_size: usize, hop_size: usize, nb_bands: usize, min_nb_freqs: usize) -> Self {
        assert!(hop_size * 2 <= fft_size);
        let mut fft = RealFftPlanner::<f32>::new();
        let frame_size = hop_size;
        let window_size = fft_size;
        let window_size_h = fft_size / 2;
        let freq_size = fft_size / 2 + 1;
        let forward = fft.plan_fft_forward(fft_size);
        let backward = fft.plan_fft_inverse(fft_size);
        let analysis_mem = vec![0.; fft_size - frame_size];
        let synthesis_mem = vec![0.; fft_size - frame_size];
        let analysis_scratch = forward.make_scratch_vec();
        let synthesis_scratch = backward.make_scratch_vec();
        let analysis_buf = forward.make_input_vec();
        let synthesis_buf = backward.make_output_vec();

        let erb = erb_fb(sr, fft_size, nb_bands, min_nb_freqs);

        let pi = std::f64::consts::PI;
        // Initialize the vorbis window: sin(pi/2*sin^2(pi*n/N))
        let mut window = vec![0.0; fft_size];
        for (i, w) in window.iter_mut().enumerate() {
            let sin = (0.5 * pi * (i as f64 + 0.5) / window_size_h as f64).sin();
            *w = (0.5 * pi * sin * sin).sin() as f32;
        }
        let wnorm = 1. / (window_size.pow(2) as f32 / (2 * frame_size) as f32);

        DFState {
            frame_size,
            window_size,
            freq_size,
            fft_forward: forward,
            fft_inverse: backward,
            erb,
            analysis_mem,
            analysis_scratch,
            analysis_buf,
            synthesis_mem,
            synthesis_scratch,
            synthesis_buf,
            window,
            wnorm,
            mean_norm_state: Vec::new(),
            unit_norm_state: Vec::new(),
        }
    }

    /// Forgets the audio of the frames before, which the STFT's overlap carries.
    pub(crate) fn reset(&mut self) {
        self.analysis_mem.fill(0.);
        self.synthesis_mem.fill(0.);
    }

    pub(crate) fn analysis(&mut self, input: &[f32], output: &mut [Complex32]) {
        debug_assert_eq!(input.len(), self.frame_size);
        debug_assert_eq!(output.len(), self.freq_size);
        frame_analysis(input, output, self)
    }

    pub(crate) fn synthesis(&mut self, input: &mut [Complex32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), self.freq_size);
        debug_assert_eq!(output.len(), self.frame_size);
        frame_synthesis(input, output, self)
    }

    /// Sets the features' normalization states to where they start.
    pub(crate) fn init_norm_states(&mut self, nb_df_freqs: usize) {
        let nb_erb = self.erb.len();
        let step = (MEAN_NORM_INIT[1] - MEAN_NORM_INIT[0]) / (nb_erb - 1) as f32;
        self.mean_norm_state = (0..nb_erb).map(|i| MEAN_NORM_INIT[0] + i as f32 * step).collect();
        let step = (UNIT_NORM_INIT[1] - UNIT_NORM_INIT[0]) / (nb_df_freqs - 1) as f32;
        self.unit_norm_state = (0..nb_df_freqs).map(|i| UNIT_NORM_INIT[0] + i as f32 * step).collect();
    }

    pub(crate) fn feat_erb(&mut self, input: &[Complex32], alpha: f32, output: &mut [f32]) {
        compute_band_corr(output, input, &self.erb); // ERB FB
        for o in output.iter_mut() {
            *o = (*o + 1e-10).log10() * 10.;
        }
        band_mean_norm_erb(output, &mut self.mean_norm_state, alpha); // Exponential mean norm
    }

    /// The unit-normalized complex features, all real parts first, then all imaginary ones.
    pub(crate) fn feat_cplx_t(&mut self, input: &[Complex32], alpha: f32, output: &mut [f32]) {
        band_unit_norm_t(input, &mut self.unit_norm_state, alpha, output)
    }

    pub(crate) fn apply_mask(&self, output: &mut [Complex32], gains: &[f32]) {
        apply_interp_band_gain(output, gains, &self.erb)
    }
}

fn band_mean_norm_erb(xs: &mut [f32], state: &mut [f32], alpha: f32) {
    debug_assert_eq!(xs.len(), state.len());
    for (x, s) in xs.iter_mut().zip(state.iter_mut()) {
        *s = *x * (1. - alpha) + *s * alpha;
        *x -= *s;
        *x /= 40.;
    }
}

/// Band unit norm, with all real parts in `out` first, then all imaginary ones, as the encoder
/// takes them.
fn band_unit_norm_t(xs: &[Complex32], state: &mut [f32], alpha: f32, out: &mut [f32]) {
    debug_assert_eq!(xs.len(), state.len());
    debug_assert_eq!(xs.len(), out.len() / 2);
    let (o_re, o_im) = out.split_at_mut(xs.len());
    for (((x, s), o_re), o_im) in xs.iter().zip(state.iter_mut()).zip(o_re.iter_mut()).zip(o_im.iter_mut()) {
        *s = x.norm() * (1. - alpha) + *s * alpha;
        // The max() guards against NaN on sustained digital silence, where the exponential
        // state underflows to 0 after ~1.5 min (0.0/0.0).
        let norm = s.max(1e-10).sqrt();
        *o_re = x.re / norm;
        *o_im = x.im / norm;
    }
}

fn compute_band_corr(out: &mut [f32], x: &[Complex32], erb_fb: &[usize]) {
    for y in out.iter_mut() {
        *y = 0.0;
    }
    debug_assert_eq!(erb_fb.len(), out.len());

    let mut bcsum = 0;
    for (&band_size, out_b) in erb_fb.iter().zip(out.iter_mut()) {
        let k = 1. / band_size as f32;
        for j in 0..band_size {
            let idx = bcsum + j;
            *out_b += (x[idx].re * x[idx].re + x[idx].im * x[idx].im) * k;
        }
        bcsum += band_size;
    }
}

fn apply_interp_band_gain(out: &mut [Complex32], band_e: &[f32], erb_fb: &[usize]) {
    let mut bcsum = 0;
    for (&band_size, &b) in erb_fb.iter().zip(band_e.iter()) {
        for j in 0..band_size {
            let idx = bcsum + j;
            out[idx] *= b;
        }
        bcsum += band_size;
    }
}

fn frame_analysis(input: &[f32], output: &mut [Complex32], state: &mut DFState) {
    debug_assert_eq!(input.len(), state.frame_size);
    debug_assert_eq!(output.len(), state.freq_size);

    // Take the persistent buffer to sidestep split borrows; moved back below.
    let mut buf = std::mem::take(&mut state.analysis_buf);
    // First part of the window on the previous frame
    let (buf_first, buf_second) = buf.split_at_mut(state.window_size - state.frame_size);
    let (window_first, window_second) = state.window.split_at(state.window_size - state.frame_size);
    let analysis_split = state.analysis_mem.len() - state.frame_size;
    for ((&y, &w), x) in state.analysis_mem.iter().zip(window_first.iter()).zip(buf_first.iter_mut()) {
        *x = y * w;
    }
    // Second part of the window on the new input frame
    for ((&y, &w), x) in input.iter().zip(window_second.iter()).zip(buf_second.iter_mut()) {
        *x = y * w;
    }
    // Shift analysis_mem
    if analysis_split > 0 {
        // hop_size is < window_size / 2
        state.analysis_mem.rotate_left(state.frame_size);
    }
    // Copy input to analysis_mem for next iteration
    for (x, &y) in state.analysis_mem[analysis_split..].iter_mut().zip(input) {
        *x = y
    }
    state
        .fft_forward
        .process_with_scratch(&mut buf, output, &mut state.analysis_scratch)
        .expect("the buffers have the lengths the FFT was planned for");
    state.analysis_buf = buf;
    // Apply normalization in analysis only
    let norm = state.wnorm;
    for x in output.iter_mut() {
        *x *= norm;
    }
}

fn frame_synthesis(input: &mut [Complex32], output: &mut [f32], state: &mut DFState) {
    // Take the persistent buffer to sidestep split borrows; moved back below.
    let mut x = std::mem::take(&mut state.synthesis_buf);
    match state.fft_inverse.process_with_scratch(input, &mut x, &mut state.synthesis_scratch) {
        // The imaginary parts of the DC and Nyquist bins, which a real signal hasn't, are ignored.
        Err(realfft::FftError::InputValues(_, _)) | Ok(()) => (),
        Err(error) => panic!("the buffers have the lengths the FFT was planned for: {error:?}"),
    }
    for (x, &w) in x.iter_mut().zip(&state.window) {
        *x *= w;
    }
    let (x_first, x_second) = x.split_at(state.frame_size);
    for ((&xi, &mem), out) in x_first.iter().zip(state.synthesis_mem.iter()).zip(output.iter_mut()) {
        *out = xi + mem;
    }

    let split = state.synthesis_mem.len() - state.frame_size;
    if split > 0 {
        state.synthesis_mem.rotate_left(state.frame_size);
    }
    let (s_first, s_second) = state.synthesis_mem.split_at_mut(split);
    let (xs_first, xs_second) = x_second.split_at(split);
    for (&xi, mem) in xs_first.iter().zip(s_first.iter_mut()) {
        // Overlap add for next frame
        *mem += xi;
    }
    for (&xi, mem) in xs_second.iter().zip(s_second.iter_mut()) {
        // Override left shifted buffer
        *mem = xi;
    }
    state.synthesis_buf = x;
}

fn post_filter(noisy: &[Complex32], enh: &mut [Complex32], beta: f32) {
    let beta_p1 = beta + 1.;
    let eps = 1e-12;
    let pi = std::f32::consts::PI;
    let mut g = [0.0; 4];
    let mut g_sin = [0.0; 4];
    let mut pf = [0.0; 4];
    for (n, e) in noisy.as_chunks::<4>().0.iter().zip(enh.as_chunks_mut::<4>().0.iter_mut()) {
        for i in 0..4 {
            g[i] = (e[i].norm() / (n[i].norm() + eps)).min(1.).max(eps);
            g_sin[i] = g[i] * (g[i] * pi / 2.0).sin();
            pf[i] = (beta_p1 * g[i] / (1. + beta * (g[i] / g_sin[i]).powi(2))) / g[i];
            e[i] *= pf[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mask_scales_each_erb_band_s_bins_by_its_gain() {
        let (n_fft, nb_bands) = (192, 24);
        let state = DFState::new(24000, n_fft, n_fft / 2, nb_bands, 1);
        let input: Vec<Complex32> = (0..n_fft / 2 + 1).map(|i| Complex32::new(i as f32, -(i as f32))).collect();
        let mut mask = vec![1.; nb_bands];
        mask[3] = 0.3;
        mask[nb_bands - 1] = 0.5;
        let mut output = input.clone();
        state.apply_mask(&mut output, &mask);
        let mut cumsum = 0;
        for (erb_idx, erb_w) in state.erb.iter().enumerate() {
            for i in cumsum..cumsum + erb_w {
                assert_eq!(input[i] * mask[erb_idx], output[i])
            }
            cumsum += erb_w;
        }
    }

    #[test]
    fn analysis_and_synthesis_reconstruct_the_signal_a_window_later() {
        let mut state = DFState::new(48000, 960, 480, 32, 2);
        let input: Vec<f32> = (0..480 * 20).map(|n| (n as f32 * 0.01).sin() * 0.5).collect();
        let mut spectrum = vec![Complex32::default(); 481];
        let mut output = vec![0.; input.len()];
        for (frame, out) in input.as_chunks::<480>().0.iter().zip(output.as_chunks_mut::<480>().0.iter_mut()) {
            state.analysis(frame, &mut spectrum);
            state.synthesis(&mut spectrum, out);
        }
        for (n, (&out, &inp)) in output[480..].iter().zip(&input).enumerate() {
            assert!((out - inp).abs() < 1e-4, "sample {n}: {out} for {inp}");
        }
    }
}
