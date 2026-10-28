//! The enhancement of a hop around the networks: the features they take from the noisy
//! spectrum, and the ERB gains and the deep filter they return, applied to it.

use std::collections::VecDeque;

use crate::network::{DfDecoder, Encoder, ErbDecoder, DF_ORDER, NB_DF, NB_ERB};
use crate::weights::{Error, Weights};
use crate::{post_filter, Complex32, DFState};

const SAMPLE_RATE: usize = 48_000;
const FFT_SIZE: usize = 960;
const HOP_SIZE: usize = 480;
/// The fewest frequency bins an ERB band spans.
const MIN_NB_ERB_FREQS: usize = 2;
/// The hops the networks look ahead: the encoder's convolutions and the deep filter.
const LOOKAHEAD: usize = 2;
/// How much of the features' normalization a hop keeps from those before: `exp(−hop / τ)` for
/// τ = 1 s, rounded to three digits as DeepFilterNet does.
const NORM_ALPHA: f32 = 0.99;

/// How the enhancement runs.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeParams {
    /// The post filter's strength, which suppresses noise between harmonics further; 0 turns it
    /// off.
    pub post_filter_beta: f32,
    /// How far noise is attenuated at most, in dB; 100 or more means without limit, below 0.01
    /// the signal passes as it is.
    pub atten_lim_db: f32,
    /// Below this local SNR, in dB, a frame is taken for noise alone and silenced.
    pub min_db_thresh: f32,
    /// Above this local SNR, in dB, a frame is taken for clean speech and passes as it is.
    pub max_db_erb_thresh: f32,
    /// Above this local SNR, in dB, a frame gets only the ERB gains, not the deep filter.
    pub max_db_df_thresh: f32,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self { post_filter_beta: 0., atten_lim_db: 100., min_db_thresh: -10., max_db_erb_thresh: 30., max_db_df_thresh: 20. }
    }
}

/// What a frame of some local SNR gets.
enum Stages {
    /// Taken for noise alone: zero gains.
    Silence,
    /// Taken for clean speech: nothing.
    Pass,
    /// Little noise: the ERB gains.
    Gains,
    /// The ERB gains and the deep filter.
    GainsAndFilter,
}

/// The local SNRs, in dB, that decide a frame's stages; as [`RuntimeParams`] has them.
struct Thresholds {
    min_db: f32,
    max_db_erb: f32,
    max_db_df: f32,
}

impl Thresholds {
    fn stages(&self, lsnr: f32) -> Stages {
        if lsnr < self.min_db {
            Stages::Silence
        } else if lsnr > self.max_db_erb {
            Stages::Pass
        } else if lsnr > self.max_db_df {
            Stages::Gains
        } else {
            Stages::GainsAndFilter
        }
    }
}

/// DeepFilterNet3 enhancing one channel of 48 kHz audio a hop at a time.
pub struct DeepFilterNet {
    encoder: Encoder,
    erb_decoder: ErbDecoder,
    df_decoder: DfDecoder,
    post_filter_beta: f32,
    atten_lim: Option<f32>,
    thresholds: Thresholds,
    state: DFState,
    /// The spectrum of the hop analysed last, and then of the hop enhanced.
    spec: Vec<Complex32>,
    /// The encoder's inputs: the ERB features, and the complex features, all real parts first.
    feat_erb: Vec<f32>,
    feat_spec: Vec<f32>,
    /// The spectra of the latest hops, oldest first: enhanced by the gains, and as they were.
    rolling_spec_buf_y: VecDeque<Vec<Complex32>>,
    rolling_spec_buf_x: VecDeque<Vec<Complex32>>,
}

impl DeepFilterNet {
    /// The model whose weights `model` holds, as [`DEEPFILTERNET3`](crate::DEEPFILTERNET3).
    pub fn new(model: &[u8], runtime: &RuntimeParams) -> Result<Self, Error> {
        let mut weights = Weights::read(model)?;
        let encoder = Encoder::new(&mut weights)?;
        let erb_decoder = ErbDecoder::new(&mut weights)?;
        let df_decoder = DfDecoder::new(&mut weights)?;
        weights.finish()?;

        let n_freqs = FFT_SIZE / 2 + 1;
        let state = DFState::new(SAMPLE_RATE, FFT_SIZE, HOP_SIZE, NB_ERB, MIN_NB_ERB_FREQS, NB_DF);
        let mut model = Self {
            encoder,
            erb_decoder,
            df_decoder,
            post_filter_beta: 0.,
            atten_lim: None,
            thresholds: Thresholds {
                min_db: runtime.min_db_thresh,
                max_db_erb: runtime.max_db_erb_thresh,
                max_db_df: runtime.max_db_df_thresh,
            },
            state,
            spec: vec![Complex32::default(); n_freqs],
            feat_erb: vec![0.; NB_ERB],
            feat_spec: vec![0.; 2 * NB_DF],
            rolling_spec_buf_y: (0..DF_ORDER + LOOKAHEAD).map(|_| vec![Complex32::default(); n_freqs]).collect(),
            rolling_spec_buf_x: (0..DF_ORDER.max(LOOKAHEAD)).map(|_| vec![Complex32::default(); n_freqs]).collect(),
        };
        model.set_pf_beta(runtime.post_filter_beta);
        model.set_atten_lim(runtime.atten_lim_db);
        Ok(model)
    }

    /// The samples [`process`](Self::process) takes and returns at a time.
    pub fn hop_size(&self) -> usize {
        HOP_SIZE
    }

    /// How many samples later the enhanced signal comes out than the noisy one went in: the
    /// STFT's overlap and the model's lookahead.
    pub fn delay(&self) -> usize {
        FFT_SIZE - HOP_SIZE + LOOKAHEAD * HOP_SIZE
    }

    /// The post filter's strength; 0 or less turns it off.
    pub fn set_pf_beta(&mut self, beta: f32) {
        self.post_filter_beta = beta.max(0.);
    }

    /// How far noise is attenuated at most, in dB; 100 or more means without limit, below 0.01
    /// the signal passes as it is.
    pub fn set_atten_lim(&mut self, db: f32) {
        let lim = db.abs();
        self.atten_lim = if lim >= 100. {
            None
        } else if lim < 0.01 {
            Some(1.)
        } else {
            Some(10f32.powf(-lim / 20.))
        };
    }

    /// Starts anew, as if just made: forgets every earlier hop, in the networks' states as in
    /// the STFT's overlap, the lookahead's buffers and the features' normalization.
    pub fn reset(&mut self) {
        self.encoder.reset();
        self.erb_decoder.reset();
        self.df_decoder.reset();
        self.state.reset();
        self.spec.fill(Complex32::default());
        for spec in self.rolling_spec_buf_y.iter_mut().chain(self.rolling_spec_buf_x.iter_mut()) {
            spec.fill(Complex32::default());
        }
    }

    /// Enhances a hop of `noisy` samples into `enh`, [`delay`](Self::delay) samples later; both
    /// [`hop_size`](Self::hop_size) long. Returns the model's estimate of the local SNR, in dB.
    pub fn process(&mut self, noisy: &[f32], enh: &mut [f32]) -> f32 {
        assert_eq!(noisy.len(), HOP_SIZE, "a hop of noisy samples");
        assert_eq!(enh.len(), HOP_SIZE, "a hop of enhanced samples");

        // Recycle the oldest spectra for the newest instead of allocating.
        let mut front_y = self.rolling_spec_buf_y.pop_front().expect("the buffer holds DF_ORDER + LOOKAHEAD spectra");
        let mut front_x = self.rolling_spec_buf_x.pop_front().expect("the buffer holds DF_ORDER spectra or more");
        self.state.analysis(noisy, &mut self.spec);
        front_y.copy_from_slice(&self.spec);
        front_x.copy_from_slice(&self.spec);
        self.rolling_spec_buf_y.push_back(front_y);
        self.rolling_spec_buf_x.push_back(front_x);
        if self.atten_lim == Some(1.) {
            enh.copy_from_slice(noisy);
            return 35.;
        }

        self.state.feat_erb(&self.spec, NORM_ALPHA, &mut self.feat_erb);
        self.state.feat_cplx_t(&self.spec[..NB_DF], NORM_ALPHA, &mut self.feat_spec);
        let encoded = self.encoder.run(&self.feat_erb, &self.feat_spec);
        let lsnr = encoded.lsnr;
        let stages = self.thresholds.stages(lsnr);

        let enhanced = &mut self.rolling_spec_buf_y[DF_ORDER - 1];
        match stages {
            Stages::Silence => enhanced.fill(Complex32::default()),
            Stages::Gains | Stages::GainsAndFilter => self.state.apply_mask(enhanced, self.erb_decoder.run(encoded)),
            Stages::Pass => (),
        }
        // This spectrum will only be used for the upper frequecies.
        self.spec.copy_from_slice(enhanced);
        if let Stages::GainsAndFilter = stages {
            df(&self.rolling_spec_buf_x, self.df_decoder.run(encoded), &mut self.spec);
        }

        let spec_noisy = &self.rolling_spec_buf_x[DF_ORDER.max(LOOKAHEAD) - LOOKAHEAD - 1];
        if matches!(stages, Stages::Gains | Stages::GainsAndFilter) && self.post_filter_beta > 0. {
            post_filter(spec_noisy, &mut self.spec, self.post_filter_beta);
        }
        // Limit noise attenuation by mixing back some of the noisy signal
        if let Some(lim) = self.atten_lim {
            for (enhanced, noisy) in self.spec.iter_mut().zip(spec_noisy) {
                *enhanced *= 1. - lim;
                *enhanced += Complex32::from(lim) * *noisy;
            }
        }
        self.state.synthesis(&mut self.spec, enh);
        lsnr
    }
}

/// Deep filtering: the lowest [`NB_DF`] bins of `spec_out` become the sum over the latest
/// [`DF_ORDER`] noisy spectra `spec`, oldest first, weighted by `coefs`, of shape
/// `[NB_DF][DF_ORDER][2]`.
fn df(spec: &VecDeque<Vec<Complex32>>, coefs: &[f32], spec_out: &mut [Complex32]) {
    debug_assert_eq!(coefs.len(), NB_DF * DF_ORDER * 2);
    debug_assert!(spec.len() >= DF_ORDER);
    spec_out[..NB_DF].fill(Complex32::default());
    for (order, s) in spec.iter().take(DF_ORDER).enumerate() {
        for (bin, (out, &s)) in spec_out[..NB_DF].iter_mut().zip(&s[..NB_DF]).enumerate() {
            let coef = 2 * (bin * DF_ORDER + order);
            *out += s * Complex32::new(coefs[coef], coefs[coef + 1]);
        }
    }
}
