//! The model: its three networks loaded from a model archive in memory, pulsed already to run a
//! hop at a time on tract, and the enhancement of a hop around them.

use std::collections::VecDeque;
use std::io::Read;
use std::str::FromStr;
use std::sync::Arc;

use flate2::read::GzDecoder;
use ini::{Ini, Properties};
use tar::Archive;
use tract_core::prelude::*;
use tract_nnef::framework::Nnef;
use tract_pulse_opl::WithPulse;

use crate::{post_filter, Complex32, DFState};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the model archive can't be read: {0}")]
    Archive(#[from] std::io::Error),
    #[error("the model archive has no {0}")]
    MissingMember(&'static str),
    #[error("the model's config.ini can't be read: {0}")]
    Config(#[from] ini::Error),
    #[error("the model's config.ini has no section [{0}]")]
    MissingSection(&'static str),
    #[error("the model's config.ini has no {key} in [{section}]")]
    MissingKey { section: &'static str, key: &'static str },
    #[error("the model's config.ini has {key} = {value}, which isn't a valid value")]
    InvalidValue { key: &'static str, value: String },
    #[error("the model is a {0}, not a DeepFilterNet3")]
    UnsupportedModel(String),
    #[error(transparent)]
    Tract(#[from] TractError),
}

/// A model archive's networks and config, read from a `.tar.gz` such as `DeepFilterNet3.tar.gz`,
/// which `examples/convert.rs` makes: `config.ini`, and each network pulsed, as NNEF.
pub struct DfParams {
    config: Ini,
    enc: Vec<u8>,
    erb_dec: Vec<u8>,
    df_dec: Vec<u8>,
}

impl DfParams {
    pub fn from_bytes(tar_gz: &[u8]) -> Result<Self, Error> {
        let mut archive = Archive::new(GzDecoder::new(tar_gz));
        let (mut enc, mut erb_dec, mut df_dec, mut config) = (None, None, None, None);
        for entry in archive.entries()? {
            let mut file = entry?;
            let path = file.path()?.into_owned();
            let member = if path.ends_with("enc.nnef.tar") {
                &mut enc
            } else if path.ends_with("erb_dec.nnef.tar") {
                &mut erb_dec
            } else if path.ends_with("df_dec.nnef.tar") {
                &mut df_dec
            } else if path.ends_with("config.ini") {
                config = Some(Ini::read_from(&mut file)?);
                continue;
            } else {
                // A version note or the like, which inference doesn't need.
                continue;
            };
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            *member = Some(bytes);
        }
        Ok(Self {
            config: config.ok_or(Error::MissingMember("config.ini"))?,
            enc: enc.ok_or(Error::MissingMember("enc.nnef.tar"))?,
            erb_dec: erb_dec.ok_or(Error::MissingMember("erb_dec.nnef.tar"))?,
            df_dec: df_dec.ok_or(Error::MissingMember("df_dec.nnef.tar"))?,
        })
    }
}

/// How the enhancement runs.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeParams {
    /// The post filter's strength, which suppresses noise between harmonics further; 0 turns it
    /// off.
    pub post_filter_beta: f32,
    /// How far noise is attenuated at most, in dB, by mixing the noisy signal back in; 100 or
    /// more means without limit, below 0.01 the signal passes as it is.
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

/// A network ready to run, and its state between runs.
struct Network {
    plan: Arc<TypedRunnableModel>,
    state: TypedSimpleState,
}

impl Network {
    /// The network in `nnef_tar`, optimized for this machine.
    fn new(nnef: &Nnef, nnef_tar: &[u8]) -> Result<Self, Error> {
        let plan = nnef.model_for_read(&mut &nnef_tar[..])?.into_optimized()?.into_runnable()?;
        let state = plan.spawn()?;
        Ok(Self { plan, state })
    }

    fn run(&mut self, inputs: TVec<TValue>) -> Result<TVec<TValue>, Error> {
        Ok(self.state.run(inputs)?)
    }

    /// Forgets every run before, as a new state does.
    fn reset(&mut self) -> Result<(), Error> {
        self.state = self.plan.spawn()?;
        Ok(())
    }
}

/// The gains the ERB decoder gives a frame, if any.
enum Gains {
    Model(TValue),
    Zeros,
}

/// DeepFilterNet3 enhancing one channel a hop at a time.
pub struct DfTract {
    enc: Network,
    erb_dec: Network,
    df_dec: Network,
    lookahead: usize,
    fft_size: usize,
    hop_size: usize,
    nb_df: usize,
    df_order: usize,
    alpha: f32,
    post_filter: bool,
    post_filter_beta: f32,
    atten_lim: Option<f32>,
    pub min_db_thresh: f32,
    pub max_db_erb_thresh: f32,
    pub max_db_df_thresh: f32,
    state: DFState,
    /// The spectrum of the hop analysed last, and then of the hop enhanced.
    spec: Vec<Complex32>,
    /// The encoder's inputs, written in place each hop: the ERB features of shape
    /// `[1, 1, 1, nb_erb]` and the complex features of shape `[1, 2, 1, nb_df]`.
    erb_buf: Arc<Tensor>,
    cplx_buf: Arc<Tensor>,
    /// The spectra of the latest hops, oldest first: enhanced by the gains, and as they were.
    rolling_spec_buf_y: VecDeque<Vec<Complex32>>,
    rolling_spec_buf_x: VecDeque<Vec<Complex32>>,
    zero_gains: Vec<f32>,
}

impl DfTract {
    pub fn new(dfp: DfParams, rp: &RuntimeParams) -> Result<Self, Error> {
        let config = dfp.config;
        let model_cfg = section(&config, "deepfilternet")?;
        let df_cfg = section(&config, "df")?;
        let train_cfg = section(&config, "train")?;

        let sr: usize = value(df_cfg, "df", "sr")?;
        let hop_size: usize = value(df_cfg, "df", "hop_size")?;
        let fft_size: usize = value(df_cfg, "df", "fft_size")?;
        let min_nb_erb_freqs: usize = value(df_cfg, "df", "min_nb_erb_freqs")?;
        let nb_erb: usize = value(df_cfg, "df", "nb_erb")?;
        let nb_df: usize = value(df_cfg, "df", "nb_df")?;
        let df_order: usize = value(df_cfg, "df", "df_order").or_else(|_| value(model_cfg, "deepfilternet", "df_order"))?;
        let conv_lookahead: usize = value(model_cfg, "deepfilternet", "conv_lookahead")?;
        let df_lookahead: usize = value(df_cfg, "df", "df_lookahead").or_else(|_| value(model_cfg, "deepfilternet", "df_lookahead"))?;
        let alpha = match value(df_cfg, "df", "norm_alpha") {
            Ok(alpha) => alpha,
            Err(_) => calc_norm_alpha(sr, hop_size, value(df_cfg, "df", "norm_tau")?),
        };
        let model: &str = train_cfg.get("model").ok_or(Error::MissingKey { section: "train", key: "model" })?;
        if model != "deepfilternet3" {
            return Err(Error::UnsupportedModel(model.to_string()));
        }
        let lookahead = conv_lookahead.max(df_lookahead);

        let nnef = tract_nnef::nnef().with_pulse();
        let enc = Network::new(&nnef, &dfp.enc)?;
        let erb_dec = Network::new(&nnef, &dfp.erb_dec)?;
        let df_dec = Network::new(&nnef, &dfp.df_dec)?;

        let n_freqs = fft_size / 2 + 1;
        let mut state = DFState::new(sr, fft_size, hop_size, nb_erb, min_nb_erb_freqs);
        state.init_norm_states(nb_df);
        let mut m = Self {
            enc,
            erb_dec,
            df_dec,
            lookahead,
            fft_size,
            hop_size,
            nb_df,
            df_order,
            alpha,
            post_filter: false,
            post_filter_beta: 0.,
            atten_lim: None,
            min_db_thresh: rp.min_db_thresh,
            max_db_erb_thresh: rp.max_db_erb_thresh,
            max_db_df_thresh: rp.max_db_df_thresh,
            state,
            spec: vec![Complex32::default(); n_freqs],
            erb_buf: Arc::new(Tensor::zero::<f32>(&[1, 1, 1, nb_erb])?),
            cplx_buf: Arc::new(Tensor::zero::<f32>(&[1, 2, 1, nb_df])?),
            rolling_spec_buf_y: (0..df_order + conv_lookahead).map(|_| vec![Complex32::default(); n_freqs]).collect(),
            rolling_spec_buf_x: (0..df_order.max(lookahead)).map(|_| vec![Complex32::default(); n_freqs]).collect(),
            zero_gains: vec![0.; nb_erb],
        };
        m.set_pf_beta(rp.post_filter_beta);
        m.set_atten_lim(rp.atten_lim_db);
        Ok(m)
    }

    /// The samples [`process`](Self::process) takes and returns at a time.
    pub fn hop_size(&self) -> usize {
        self.hop_size
    }

    /// How many samples later the enhanced signal comes out than the noisy one went in: the
    /// STFT's overlap and the model's lookahead.
    pub fn delay(&self) -> usize {
        self.fft_size - self.hop_size + self.lookahead * self.hop_size
    }

    /// The post filter's strength; 0 or less turns it off.
    pub fn set_pf_beta(&mut self, beta: f32) {
        self.post_filter = beta > 0.;
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
    pub fn reset(&mut self) -> Result<(), Error> {
        self.enc.reset()?;
        self.erb_dec.reset()?;
        self.df_dec.reset()?;
        self.state.reset();
        self.state.init_norm_states(self.nb_df);
        self.spec.fill(Complex32::default());
        for spec in self.rolling_spec_buf_y.iter_mut().chain(self.rolling_spec_buf_x.iter_mut()) {
            spec.fill(Complex32::default());
        }
        write(&mut self.erb_buf, |floats| floats.fill(0.))?;
        write(&mut self.cplx_buf, |floats| floats.fill(0.))?;
        Ok(())
    }

    /// Enhances a hop of `noisy` samples into `enh`, [`delay`](Self::delay) samples later; both
    /// [`hop_size`](Self::hop_size) long. Returns the model's estimate of the local SNR, in dB.
    pub fn process(&mut self, noisy: &[f32], enh: &mut [f32]) -> Result<f32, Error> {
        assert_eq!(noisy.len(), self.hop_size, "a hop of noisy samples");
        assert_eq!(enh.len(), self.hop_size, "a hop of enhanced samples");

        // Recycle the oldest spectra for the newest instead of allocating.
        let mut front_y = self.rolling_spec_buf_y.pop_front().expect("the buffer holds df_order + conv_lookahead spectra");
        let mut front_x = self.rolling_spec_buf_x.pop_front().expect("the buffer holds df_order spectra or more");
        self.state.analysis(noisy, &mut self.spec);
        front_y.copy_from_slice(&self.spec);
        front_x.copy_from_slice(&self.spec);
        self.rolling_spec_buf_y.push_back(front_y);
        self.rolling_spec_buf_x.push_back(front_x);
        if self.atten_lim == Some(1.) {
            enh.copy_from_slice(noisy);
            return Ok(35.);
        }

        let (lsnr, gains, coefs) = self.process_raw()?;

        let (apply_erb, _, _) = self.apply_stages(lsnr);
        let enhanced = &mut self.rolling_spec_buf_y[self.df_order - 1];
        match &gains {
            Some(Gains::Model(gains)) => self.state.apply_mask(enhanced, gains.try_as_plain_ram()?.as_slice::<f32>()?),
            Some(Gains::Zeros) => self.state.apply_mask(enhanced, &self.zero_gains),
            None => (),
        }
        // This spectrum will only be used for the upper frequecies.
        self.spec.copy_from_slice(enhanced);
        if let Some(coefs) = coefs {
            df(&self.rolling_spec_buf_x, coefs.try_as_plain_ram()?.as_slice::<f32>()?, self.nb_df, self.df_order, &mut self.spec);
        }

        let spec_noisy = &self.rolling_spec_buf_x[self.lookahead.max(self.df_order) - self.lookahead - 1];
        if apply_erb && self.post_filter {
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
        Ok(lsnr)
    }

    /// Runs the networks on the spectrum analysed last: the local SNR, and as it calls for, the
    /// ERB gains and the deep filter's coefficients, of shape `[1, nb_df, df_order, 2]`.
    fn process_raw(&mut self) -> Result<(f32, Option<Gains>, Option<TValue>), Error> {
        let (state, spec, alpha) = (&mut self.state, &self.spec, self.alpha);
        write(&mut self.erb_buf, |floats| state.feat_erb(spec, alpha, floats))?;
        write(&mut self.cplx_buf, |floats| state.feat_cplx_t(&spec[..self.nb_df], alpha, floats))?;
        let mut enc_emb = self.enc.run(tvec!(TValue::from_const(self.erb_buf.clone()), TValue::from_const(self.cplx_buf.clone())))?;

        let lsnr_t = enc_emb.pop().expect("the encoder outputs lsnr");
        let lsnr_t = lsnr_t.try_as_plain_ram()?.as_slice::<f32>()?;
        let lsnr = lsnr_t.iter().sum::<f32>() / lsnr_t.len() as f32;
        let c0 = enc_emb.pop().expect("the encoder outputs c0");
        let emb = enc_emb.pop().expect("the encoder outputs emb");

        let (apply_gains, apply_gain_zeros, apply_df) = self.apply_stages(lsnr);
        let gains = if apply_gains {
            let (e3, e2, e1, e0) = (enc_emb.pop(), enc_emb.pop(), enc_emb.pop(), enc_emb.pop());
            let dec_input = tvec!(emb.clone(), e3.expect("e3"), e2.expect("e2"), e1.expect("e1"), e0.expect("e0"));
            Some(Gains::Model(self.erb_dec.run(dec_input)?.pop().expect("the ERB decoder outputs m")))
        } else if apply_gain_zeros {
            Some(Gains::Zeros)
        } else {
            None
        };
        let coefs = if apply_df { Some(self.df_dec.run(tvec!(emb, c0))?.pop().expect("the DF decoder outputs coefs")) } else { None };
        Ok((lsnr, gains, coefs))
    }

    /// Which stages a frame of local SNR `lsnr` gets: the ERB gains, zero gains instead, and the
    /// deep filter.
    fn apply_stages(&self, lsnr: f32) -> (bool, bool, bool) {
        if lsnr < self.min_db_thresh {
            // Only noise detected, just apply a zero mask
            (false, true, false)
        } else if lsnr > self.max_db_erb_thresh {
            // Clean speech signal detected, don't apply any processing
            (false, false, false)
        } else if lsnr > self.max_db_df_thresh {
            // Only little noisy signal detected, just apply 1st stage, skip DF stage
            (true, false, false)
        } else {
            // Regular noisy signal detected, apply 1st and 2nd stage
            (true, false, true)
        }
    }
}

/// Writes the floats an input holds in place: tract lets go of its inputs after each run, so
/// this copies nothing then.
fn write(input: &mut Arc<Tensor>, write: impl FnOnce(&mut [f32])) -> Result<(), Error> {
    write(Arc::make_mut(input).try_as_plain_ram_mut()?.as_slice_mut::<f32>()?);
    Ok(())
}

/// Deep filtering: the lowest `nb_df` bins of `spec_out` become the sum over the latest
/// `df_order` noisy spectra `spec`, oldest first, weighted by `coefs`, of shape
/// `[nb_df, df_order, 2]`.
fn df(spec: &VecDeque<Vec<Complex32>>, coefs: &[f32], nb_df: usize, df_order: usize, spec_out: &mut [Complex32]) {
    let coefs: &[Complex32] = bytemuck::cast_slice(coefs);
    debug_assert_eq!(coefs.len(), nb_df * df_order);
    debug_assert!(spec.len() >= df_order);
    spec_out[..nb_df].fill(Complex32::default());
    for (order, s) in spec.iter().take(df_order).enumerate() {
        for (bin, (out, &s)) in spec_out[..nb_df].iter_mut().zip(&s[..nb_df]).enumerate() {
            *out += s * coefs[bin * df_order + order];
        }
    }
}

fn section<'a>(config: &'a Ini, name: &'static str) -> Result<&'a Properties, Error> {
    config.section(Some(name)).ok_or(Error::MissingSection(name))
}

fn value<T: FromStr>(section: &Properties, name: &'static str, key: &'static str) -> Result<T, Error> {
    let value = section.get(key).ok_or(Error::MissingKey { section: name, key })?;
    value.parse().map_err(|_| Error::InvalidValue { key, value: value.to_string() })
}

fn calc_norm_alpha(sr: usize, hop_size: usize, tau: f32) -> f32 {
    let dt = hop_size as f32 / sr as f32;
    let alpha = f32::exp(-dt / tau);
    let mut a = 1.0;
    let mut precision = 3;
    while a >= 1.0 {
        a = (alpha * 10i32.pow(precision) as f32).round() / 10i32.pow(precision) as f32;
        precision += 1;
    }
    a
}
