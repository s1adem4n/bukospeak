//! DeepFilterNet3's three networks, run a hop at a time: the encoder, the decoder of the ERB
//! gains and the decoder of the deep filter's coefficients. Each keeps what it needs of earlier
//! hops, the frames its convolutions look back on and its GRUs' states, and allocates nothing
//! once built.
//!
//! Feature maps are channel-major, `[channel][frequency]`, as in the ONNX networks; the weights
//! keep the ONNX layouts too.

use crate::weights::{Error, Weights};

/// The ERB bands, which the encoder's first convolution and the gains span.
pub(crate) const NB_ERB: usize = 32;
/// The lowest frequency bins, which the deep filter spans.
pub(crate) const NB_DF: usize = 96;
/// The spectra the deep filter weighs per bin, the current and those before.
pub(crate) const DF_ORDER: usize = 5;
/// The channels of the convolutions.
const CH: usize = 64;
/// The GRUs' hidden state.
const HIDDEN: usize = 256;
/// The embedding the encoder gives the decoders.
const EMB: usize = 512;
/// The local SNR's range, which the encoder's sigmoid is scaled to, in dB.
const LSNR_MIN: f32 = -15.;
const LSNR_MAX: f32 = 35.;

/// The sum of `a` and `b`'s products, in eight lanes the compiler vectorizes.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let (a_chunks, a_rest) = a.as_chunks::<8>();
    let (b_chunks, b_rest) = b.as_chunks::<8>();
    let mut lanes = [0.; 8];
    for (a, b) in a_chunks.iter().zip(b_chunks) {
        for lane in 0..8 {
            lanes[lane] += a[lane] * b[lane];
        }
    }
    lanes.iter().sum::<f32>() + a_rest.iter().zip(b_rest).map(|(a, b)| a * b).sum::<f32>()
}

/// Adds `x`'s multiple `a` to `y`.
fn axpy(a: f32, x: &[f32], y: &mut [f32]) {
    debug_assert_eq!(x.len(), y.len());
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

fn relu(xs: &mut [f32]) {
    xs.iter_mut().for_each(|x| *x = x.max(0.));
}

fn sigmoid(x: f32) -> f32 {
    1. / (1. + (-x).exp())
}

/// A convolution's weights and bias, of `outputs × weights` and `outputs` values.
struct Conv {
    weight: Vec<f32>,
    bias: Vec<f32>,
}

impl Conv {
    fn new(
        weights: &mut Weights,
        weight: &'static str,
        bias: Option<&'static str>,
        outputs: usize,
        per_output: usize,
    ) -> Result<Self, Error> {
        let bias = match bias {
            Some(bias) => weights.take(bias, outputs)?,
            None => vec![0.; outputs],
        };
        Ok(Self { weight: weights.take(weight, outputs * per_output)?, bias })
    }

    /// A 1×1 convolution across all channels of `x`, `[inputs][f]`, into `out`, `[outputs][f]`.
    fn pointwise(&self, x: &[f32], out: &mut [f32], f: usize) {
        let inputs = x.len() / f;
        for ((out, weights), &bias) in out.chunks_exact_mut(f).zip(self.weight.chunks_exact(inputs)).zip(&self.bias) {
            out.fill(bias);
            for (&weight, x) in weights.iter().zip(x.chunks_exact(f)) {
                axpy(weight, x, out);
            }
        }
    }

    /// A 1×1 convolution of each channel of `x` on its own.
    fn scale(&self, x: &[f32], out: &mut [f32], f: usize) {
        for (((out, x), &weight), &bias) in out.chunks_exact_mut(f).zip(x.chunks_exact(f)).zip(&self.weight).zip(&self.bias) {
            for (out, x) in out.iter_mut().zip(x) {
                *out = weight * x + bias;
            }
        }
    }

    /// A 1×3 convolution of each channel of `x`, `[channels][f_in]`, on its own, along the
    /// frequencies, which are padded with a zero on each side; taking every `stride`th.
    fn depthwise(&self, x: &[f32], out: &mut [f32], f_in: usize, stride: usize) {
        let f_out = out.len() / (x.len() / f_in);
        for ((out, x), kernel) in out.chunks_exact_mut(f_out).zip(x.chunks_exact(f_in)).zip(self.weight.as_chunks::<3>().0) {
            for (fo, out) in out.iter_mut().enumerate() {
                let centre = fo * stride;
                *out =
                    kernel.iter().enumerate().filter_map(|(k, w)| (centre + k).checked_sub(1).and_then(|f| x.get(f)).map(|x| w * x)).sum();
            }
        }
    }

    /// The transposed 1×3 convolution of each channel of `x` on its own, which doubles its
    /// frequencies: the inverse of [`depthwise`](Self::depthwise) with a stride of 2.
    fn depthwise_transposed(&self, x: &[f32], out: &mut [f32], f_in: usize) {
        let f_out = 2 * f_in;
        for ((out, x), kernel) in out.chunks_exact_mut(f_out).zip(x.chunks_exact(f_in)).zip(self.weight.as_chunks::<3>().0) {
            out.fill(0.);
            for (i, &x) in x.iter().enumerate() {
                for (k, w) in kernel.iter().enumerate() {
                    if let Some(out) = (2 * i + k).checked_sub(1).and_then(|o| out.get_mut(o)) {
                        *out += w * x;
                    }
                }
            }
        }
    }
}

/// A 1×3 depthwise convolution followed by a pointwise one and a ReLU: one of the encoder's
/// and the ERB decoder's layers.
struct Separable {
    depthwise: Conv,
    pointwise: Conv,
}

impl Separable {
    fn new(weights: &mut Weights, depthwise: &'static str, pointwise: [&'static str; 2]) -> Result<Self, Error> {
        Ok(Self {
            depthwise: Conv::new(weights, depthwise, None, CH, 3)?,
            pointwise: Conv::new(weights, pointwise[0], Some(pointwise[1]), CH, CH)?,
        })
    }

    /// Runs the layer on `x` of `f_in` frequencies, with `scratch` as large as `out`.
    fn run(&self, x: &[f32], scratch: &mut [f32], out: &mut [f32], f_in: usize, stride: usize) {
        self.depthwise.depthwise(x, scratch, f_in, stride);
        self.pointwise.pointwise(scratch, out, out.len() / CH);
        relu(out);
    }

    /// Runs the layer with the depthwise convolution transposed, doubling the frequencies.
    fn run_transposed(&self, x: &[f32], scratch: &mut [f32], out: &mut [f32], f_in: usize) {
        self.depthwise.depthwise_transposed(x, scratch, f_in);
        self.pointwise.pointwise(scratch, out, 2 * f_in);
        relu(out);
    }
}

/// A linear layer split into groups, each of which maps its slice of the input to its slice of
/// the output: weights `[groups][inputs][outputs]`, no bias.
struct GroupedLinear {
    weight: Vec<f32>,
    inputs: usize,
    outputs: usize,
}

impl GroupedLinear {
    fn new(weights: &mut Weights, name: &'static str, groups: usize, inputs: usize, outputs: usize) -> Result<Self, Error> {
        Ok(Self { weight: weights.take(name, groups * inputs * outputs)?, inputs, outputs })
    }

    fn run(&self, x: &[f32], out: &mut [f32]) {
        let group = self.inputs * self.outputs;
        for ((out, x), weights) in out.chunks_exact_mut(self.outputs).zip(x.chunks_exact(self.inputs)).zip(self.weight.chunks_exact(group))
        {
            out.fill(0.);
            for (&x, weights) in x.iter().zip(weights.chunks_exact(self.outputs)) {
                axpy(x, weights, out);
            }
        }
    }
}

/// A GRU of [`HIDDEN`] units as ONNX has it, gates in the order update, reset, new, with the
/// reset gate applied after the recurrent weights; and its state.
struct Gru {
    /// `[3 · HIDDEN][inputs]`.
    input: Vec<f32>,
    /// `[3 · HIDDEN][HIDDEN]`.
    recurrent: Vec<f32>,
    input_bias: Vec<f32>,
    recurrent_bias: Vec<f32>,
    state: Vec<f32>,
    /// The input's and the state's contributions to the gates, for each step.
    from_input: Vec<f32>,
    from_state: Vec<f32>,
}

impl Gru {
    fn new(weights: &mut Weights, names: [&'static str; 3], inputs: usize) -> Result<Self, Error> {
        let gates = 3 * HIDDEN;
        let mut input_bias = weights.take(names[2], 2 * gates)?;
        let recurrent_bias = input_bias.split_off(gates);
        Ok(Self {
            input: weights.take(names[0], gates * inputs)?,
            recurrent: weights.take(names[1], gates * HIDDEN)?,
            input_bias,
            recurrent_bias,
            state: vec![0.; HIDDEN],
            from_input: vec![0.; gates],
            from_state: vec![0.; gates],
        })
    }

    /// Takes the next input and returns the new state.
    fn step(&mut self, x: &[f32]) -> &[f32] {
        let inputs = x.len();
        for ((gate, weights), bias) in self.from_input.iter_mut().zip(self.input.chunks_exact(inputs)).zip(&self.input_bias) {
            *gate = dot(weights, x) + bias;
        }
        for ((gate, weights), bias) in self.from_state.iter_mut().zip(self.recurrent.as_chunks::<HIDDEN>().0).zip(&self.recurrent_bias) {
            *gate = dot(weights, &self.state) + bias;
        }
        let (x_update, x_rest) = self.from_input.split_at(HIDDEN);
        let (x_reset, x_new) = x_rest.split_at(HIDDEN);
        let (h_update, h_rest) = self.from_state.split_at(HIDDEN);
        let (h_reset, h_new) = h_rest.split_at(HIDDEN);
        for (j, state) in self.state.iter_mut().enumerate() {
            let update = sigmoid(x_update[j] + h_update[j]);
            let reset = sigmoid(x_reset[j] + h_reset[j]);
            let new = (x_new[j] + reset * h_new[j]).tanh();
            *state = (1. - update) * new + update * *state;
        }
        &self.state
    }

    fn reset(&mut self) {
        self.state.fill(0.);
    }
}

/// The frames a convolution along time looks back on, oldest first, and the current one last.
struct History {
    frames: Vec<f32>,
    frame: usize,
}

impl History {
    fn new(frames: usize, frame: usize) -> Self {
        Self { frames: vec![0.; frames * frame], frame }
    }

    /// Drops the oldest frame for `current`.
    fn push(&mut self, current: &[f32]) {
        self.frames.copy_within(self.frame.., 0);
        let start = self.frames.len() - self.frame;
        self.frames[start..].copy_from_slice(current);
    }

    fn frames(&self) -> std::slice::ChunksExact<'_, f32> {
        self.frames.chunks_exact(self.frame)
    }

    fn reset(&mut self) {
        self.frames.fill(0.);
    }
}

/// Writes `x`, `[CH][f]`, frequency-major into `out`, `[f][CH]`, as the networks flatten their
/// feature maps.
fn flatten(x: &[f32], out: &mut [f32], f: usize) {
    for (c, x) in x.chunks_exact(f).enumerate() {
        for (i, &x) in x.iter().enumerate() {
            out[i * CH + c] = x;
        }
    }
}

/// What the encoder makes of a hop, for the decoders.
pub(crate) struct Encoded {
    /// The ERB pathway's feature maps, for the ERB decoder's skip connections: `[CH][32]`,
    /// `[CH][16]`, `[CH][8]` and `[CH][8]`.
    e0: Vec<f32>,
    e1: Vec<f32>,
    e2: Vec<f32>,
    e3: Vec<f32>,
    /// The complex pathway's first feature map, `[CH][NB_DF]`, for the DF decoder.
    c0: Vec<f32>,
    emb: Vec<f32>,
    /// The local SNR, in dB.
    pub(crate) lsnr: f32,
}

pub(crate) struct Encoder {
    erb_conv0: Conv,
    erb_conv1: Separable,
    erb_conv2: Separable,
    erb_conv3: Separable,
    df_conv0: Conv,
    df_conv0_pointwise: Conv,
    df_conv1: Separable,
    df_fc_emb: GroupedLinear,
    linear_in: GroupedLinear,
    gru: Gru,
    linear_out: GroupedLinear,
    lsnr_weight: Vec<f32>,
    lsnr_bias: f32,
    /// The ERB features of the last three hops, and the complex features, `[2][NB_DF]` each.
    erb: History,
    spec: History,
    out: Encoded,
    scratch: Vec<f32>,
    c1: Vec<f32>,
    flat: Vec<f32>,
    emb_erb: Vec<f32>,
    emb_df: Vec<f32>,
    gru_in: Vec<f32>,
}

impl Encoder {
    pub(crate) fn new(weights: &mut Weights) -> Result<Self, Error> {
        Ok(Self {
            erb_conv0: Conv::new(weights, "enc.erb_conv0.weight", Some("enc.erb_conv0.bias"), CH, 9)?,
            erb_conv1: Separable::new(weights, "enc.erb_conv1.depthwise", ["enc.erb_conv1.pointwise", "enc.erb_conv1.bias"])?,
            erb_conv2: Separable::new(weights, "enc.erb_conv2.depthwise", ["enc.erb_conv2.pointwise", "enc.erb_conv2.bias"])?,
            erb_conv3: Separable::new(weights, "enc.erb_conv3.depthwise", ["enc.erb_conv3.pointwise", "enc.erb_conv3.bias"])?,
            df_conv0: Conv::new(weights, "enc.df_conv0.weight", None, CH, 9)?,
            df_conv0_pointwise: Conv::new(weights, "enc.df_conv0.pointwise", Some("enc.df_conv0.bias"), CH, CH)?,
            df_conv1: Separable::new(weights, "enc.df_conv1.depthwise", ["enc.df_conv1.pointwise", "enc.df_conv1.bias"])?,
            df_fc_emb: GroupedLinear::new(weights, "enc.df_fc_emb", 32, 96, 16)?,
            linear_in: GroupedLinear::new(weights, "enc.emb_gru.linear_in", 16, 32, 16)?,
            gru: Gru::new(weights, ["enc.emb_gru.gru.input", "enc.emb_gru.gru.recurrent", "enc.emb_gru.gru.bias"], HIDDEN)?,
            linear_out: GroupedLinear::new(weights, "enc.emb_gru.linear_out", 16, 16, 32)?,
            lsnr_weight: weights.take("enc.lsnr_fc.weight", EMB)?,
            lsnr_bias: weights.take("enc.lsnr_fc.bias", 1)?[0],
            erb: History::new(3, NB_ERB),
            spec: History::new(3, 2 * NB_DF),
            out: Encoded {
                e0: vec![0.; CH * NB_ERB],
                e1: vec![0.; CH * NB_ERB / 2],
                e2: vec![0.; CH * NB_ERB / 4],
                e3: vec![0.; CH * NB_ERB / 4],
                c0: vec![0.; CH * NB_DF],
                emb: vec![0.; EMB],
                lsnr: 0.,
            },
            scratch: vec![0.; CH * NB_DF],
            c1: vec![0.; CH * NB_DF / 2],
            flat: vec![0.; CH * NB_DF / 2],
            emb_erb: vec![0.; EMB],
            emb_df: vec![0.; EMB],
            gru_in: vec![0.; HIDDEN],
        })
    }

    /// Encodes the next hop's ERB features, `[NB_ERB]`, and complex features, all real parts
    /// and then all imaginary ones, `[2][NB_DF]`.
    pub(crate) fn run(&mut self, erb: &[f32], spec: &[f32]) -> &Encoded {
        self.erb.push(erb);
        self.spec.push(spec);
        let out = &mut self.out;

        // The ERB pathway: a 3×3 convolution over the last three hops, then three separable
        // ones, the first two halving the frequencies.
        conv3x3(&self.erb_conv0, &self.erb, 1, &mut out.e0, NB_ERB);
        relu(&mut out.e0);
        self.erb_conv1.run(&out.e0, &mut self.scratch[..out.e1.len()], &mut out.e1, NB_ERB, 2);
        self.erb_conv2.run(&out.e1, &mut self.scratch[..out.e2.len()], &mut out.e2, NB_ERB / 2, 2);
        self.erb_conv3.run(&out.e2, &mut self.scratch[..out.e3.len()], &mut out.e3, NB_ERB / 4, 1);

        // The complex pathway: real and imaginary parts convolved apart, mixed pointwise, then
        // a separable convolution halving the frequencies.
        conv3x3(&self.df_conv0, &self.spec, 2, &mut self.scratch, NB_DF);
        self.df_conv0_pointwise.pointwise(&self.scratch, &mut out.c0, NB_DF);
        relu(&mut out.c0);
        self.df_conv1.run(&out.c0, &mut self.scratch[..self.c1.len()], &mut self.c1, NB_DF, 2);
        flatten(&self.c1, &mut self.flat, NB_DF / 2);
        self.df_fc_emb.run(&self.flat, &mut self.emb_df);
        relu(&mut self.emb_df);

        // Both pathways summed, through the GRU.
        flatten(&out.e3, &mut self.emb_erb, NB_ERB / 4);
        for (erb, df) in self.emb_erb.iter_mut().zip(&self.emb_df) {
            *erb += df;
        }
        self.linear_in.run(&self.emb_erb, &mut self.gru_in);
        relu(&mut self.gru_in);
        let state = self.gru.step(&self.gru_in);
        self.linear_out.run(state, &mut out.emb);
        relu(&mut out.emb);

        out.lsnr = sigmoid(dot(&self.lsnr_weight, &out.emb) + self.lsnr_bias) * (LSNR_MAX - LSNR_MIN) + LSNR_MIN;
        out
    }

    pub(crate) fn reset(&mut self) {
        self.erb.reset();
        self.spec.reset();
        self.gru.reset();
    }
}

/// The encoder's first convolutions: 3×3, over the last three hops of `history`, each of
/// `groups` channels of `f` frequencies, padded with a zero on each side; each group of output
/// channels convolving one input channel.
fn conv3x3(conv: &Conv, history: &History, groups: usize, out: &mut [f32], f: usize) {
    let per_group = CH / groups;
    for (o, ((out, kernel), &bias)) in out.chunks_exact_mut(f).zip(conv.weight.as_chunks::<9>().0).zip(&conv.bias).enumerate() {
        out.fill(bias);
        let channel = o / per_group;
        for (frame, row) in history.frames().zip(kernel.as_chunks::<3>().0) {
            let x = &frame[channel * f..(channel + 1) * f];
            // The centre tap over all frequencies, the side taps over all but the edge.
            axpy(row[1], x, out);
            axpy(row[0], &x[..f - 1], &mut out[1..]);
            axpy(row[2], &x[1..], &mut out[..f - 1]);
        }
    }
}

pub(crate) struct ErbDecoder {
    linear_in: GroupedLinear,
    gru0: Gru,
    gru1: Gru,
    linear_out: GroupedLinear,
    conv3p: Conv,
    convt3: Separable,
    conv2p: Conv,
    convt2: Separable,
    conv1p: Conv,
    convt1: Separable,
    conv0p: Conv,
    conv0_out: Conv,
    gru_in: Vec<f32>,
    emb: Vec<f32>,
    skip: Vec<f32>,
    sum: Vec<f32>,
    scratch: Vec<f32>,
    d3: Vec<f32>,
    d2: Vec<f32>,
    d1: Vec<f32>,
    gains: Vec<f32>,
}

impl ErbDecoder {
    pub(crate) fn new(weights: &mut Weights) -> Result<Self, Error> {
        Ok(Self {
            linear_in: GroupedLinear::new(weights, "erb_dec.emb_gru.linear_in", 16, 32, 16)?,
            gru0: Gru::new(weights, ["erb_dec.emb_gru.gru0.input", "erb_dec.emb_gru.gru0.recurrent", "erb_dec.emb_gru.gru0.bias"], HIDDEN)?,
            gru1: Gru::new(weights, ["erb_dec.emb_gru.gru1.input", "erb_dec.emb_gru.gru1.recurrent", "erb_dec.emb_gru.gru1.bias"], HIDDEN)?,
            linear_out: GroupedLinear::new(weights, "erb_dec.emb_gru.linear_out", 16, 16, 32)?,
            conv3p: Conv::new(weights, "erb_dec.conv3p.weight", Some("erb_dec.conv3p.bias"), CH, 1)?,
            convt3: Separable::new(weights, "erb_dec.convt3.depthwise", ["erb_dec.convt3.pointwise", "erb_dec.convt3.bias"])?,
            conv2p: Conv::new(weights, "erb_dec.conv2p.weight", Some("erb_dec.conv2p.bias"), CH, 1)?,
            convt2: Separable::new(weights, "erb_dec.convt2.depthwise", ["erb_dec.convt2.pointwise", "erb_dec.convt2.bias"])?,
            conv1p: Conv::new(weights, "erb_dec.conv1p.weight", Some("erb_dec.conv1p.bias"), CH, 1)?,
            convt1: Separable::new(weights, "erb_dec.convt1.depthwise", ["erb_dec.convt1.pointwise", "erb_dec.convt1.bias"])?,
            conv0p: Conv::new(weights, "erb_dec.conv0p.weight", Some("erb_dec.conv0p.bias"), CH, 1)?,
            conv0_out: Conv::new(weights, "erb_dec.conv0_out.weight", Some("erb_dec.conv0_out.bias"), 1, 3 * CH)?,
            gru_in: vec![0.; HIDDEN],
            emb: vec![0.; EMB],
            skip: vec![0.; CH * NB_ERB],
            sum: vec![0.; CH * NB_ERB],
            scratch: vec![0.; CH * NB_ERB],
            d3: vec![0.; CH * NB_ERB / 4],
            d2: vec![0.; CH * NB_ERB / 2],
            d1: vec![0.; CH * NB_ERB],
            gains: vec![0.; NB_ERB],
        })
    }

    /// The gains of the ERB bands, `[NB_ERB]`, for an encoded hop.
    pub(crate) fn run(&mut self, encoded: &Encoded) -> &[f32] {
        self.linear_in.run(&encoded.emb, &mut self.gru_in);
        relu(&mut self.gru_in);
        let state = self.gru0.step(&self.gru_in);
        let state = self.gru1.step(state);
        self.linear_out.run(state, &mut self.emb);
        relu(&mut self.emb);

        // The embedding as a feature map, `[CH][NB_ERB / 4]`, then up the encoder's levels,
        // each adding the encoder's map of its level through a 1×1 convolution.
        let f3 = NB_ERB / 4;
        for (c, sum) in self.sum[..CH * f3].chunks_exact_mut(f3).enumerate() {
            for (f, sum) in sum.iter_mut().enumerate() {
                *sum = self.emb[f * CH + c];
            }
        }
        add_skip(&self.conv3p, &encoded.e3, &mut self.skip, &mut self.sum, f3);
        self.convt3.run(&self.sum[..CH * f3], &mut self.scratch[..CH * f3], &mut self.d3, f3, 1);

        self.sum[..CH * f3].copy_from_slice(&self.d3);
        add_skip(&self.conv2p, &encoded.e2, &mut self.skip, &mut self.sum, f3);
        self.convt2.run_transposed(&self.sum[..CH * f3], &mut self.scratch[..CH * 2 * f3], &mut self.d2, f3);

        let f1 = NB_ERB / 2;
        self.sum[..CH * f1].copy_from_slice(&self.d2);
        add_skip(&self.conv1p, &encoded.e1, &mut self.skip, &mut self.sum, f1);
        self.convt1.run_transposed(&self.sum[..CH * f1], &mut self.scratch, &mut self.d1, f1);

        self.sum.copy_from_slice(&self.d1);
        add_skip(&self.conv0p, &encoded.e0, &mut self.skip, &mut self.sum, NB_ERB);
        // The last convolution, 1×3 across all channels into one, through a sigmoid.
        let bias = self.conv0_out.bias[0];
        self.gains.fill(bias);
        for (x, kernel) in self.sum.as_chunks::<NB_ERB>().0.iter().zip(self.conv0_out.weight.as_chunks::<3>().0) {
            axpy(kernel[1], x, &mut self.gains);
            axpy(kernel[0], &x[..NB_ERB - 1], &mut self.gains[1..]);
            axpy(kernel[2], &x[1..], &mut self.gains[..NB_ERB - 1]);
        }
        self.gains.iter_mut().for_each(|gain| *gain = sigmoid(*gain));
        &self.gains
    }

    pub(crate) fn reset(&mut self) {
        self.gru0.reset();
        self.gru1.reset();
    }
}

/// Adds the encoder's map `e`, `[CH][f]`, through `conv` and a ReLU, to `sum`, `[CH][f]`.
fn add_skip(conv: &Conv, e: &[f32], skip: &mut [f32], sum: &mut [f32], f: usize) {
    let skip = &mut skip[..CH * f];
    conv.scale(e, skip, f);
    for (sum, skip) in sum[..CH * f].iter_mut().zip(skip.iter()) {
        *sum += skip.max(0.);
    }
}

pub(crate) struct DfDecoder {
    linear_in: GroupedLinear,
    gru0: Gru,
    gru1: Gru,
    skip: GroupedLinear,
    out: GroupedLinear,
    convp: Conv,
    convp_pointwise: Conv,
    /// The encoder's `c0` of the last [`CONVP_FRAMES`] hops this decoder ran on.
    c0: History,
    gru_in: Vec<f32>,
    hidden: Vec<f32>,
    convp_out: Vec<f32>,
    convp_mixed: Vec<f32>,
    coefs: Vec<f32>,
}

/// The coefficients per bin: each of the [`DF_ORDER`] spectra's, real and imaginary.
const COEFS: usize = 2 * DF_ORDER;
/// The hops the DF decoder's convolution along time spans.
const CONVP_FRAMES: usize = 5;

impl DfDecoder {
    pub(crate) fn new(weights: &mut Weights) -> Result<Self, Error> {
        Ok(Self {
            linear_in: GroupedLinear::new(weights, "df_dec.df_gru.linear_in", 8, 64, 32)?,
            gru0: Gru::new(weights, ["df_dec.df_gru.gru0.input", "df_dec.df_gru.gru0.recurrent", "df_dec.df_gru.gru0.bias"], HIDDEN)?,
            gru1: Gru::new(weights, ["df_dec.df_gru.gru1.input", "df_dec.df_gru.gru1.recurrent", "df_dec.df_gru.gru1.bias"], HIDDEN)?,
            skip: GroupedLinear::new(weights, "df_dec.df_skip", 16, 32, 16)?,
            out: GroupedLinear::new(weights, "df_dec.df_out", 16, 16, 60)?,
            convp: Conv::new(weights, "df_dec.df_convp.weight", None, COEFS, CH / 2 * CONVP_FRAMES)?,
            convp_pointwise: Conv::new(weights, "df_dec.df_convp.pointwise", Some("df_dec.df_convp.bias"), COEFS, COEFS)?,
            c0: History::new(CONVP_FRAMES, CH * NB_DF),
            gru_in: vec![0.; HIDDEN],
            hidden: vec![0.; HIDDEN],
            convp_out: vec![0.; COEFS * NB_DF],
            convp_mixed: vec![0.; COEFS * NB_DF],
            coefs: vec![0.; NB_DF * COEFS],
        })
    }

    /// The deep filter's coefficients for an encoded hop: `[NB_DF][DF_ORDER][2]`, the spectra
    /// oldest first, real and imaginary parts.
    pub(crate) fn run(&mut self, encoded: &Encoded) -> &[f32] {
        self.c0.push(&encoded.c0);
        self.linear_in.run(&encoded.emb, &mut self.gru_in);
        relu(&mut self.gru_in);
        let state = self.gru0.step(&self.gru_in);
        let state = self.gru1.step(state);
        self.skip.run(&encoded.emb, &mut self.hidden);
        for (hidden, state) in self.hidden.iter_mut().zip(state) {
            *hidden += state;
        }
        self.out.run(&self.hidden, &mut self.coefs);
        self.coefs.iter_mut().for_each(|coef| *coef = coef.tanh());

        // The convolution of the last hops' `c0` along time, each half of the channels into half
        // of the coefficients, mixed pointwise, added to the coefficients.
        let channels = CH / 2;
        for (o, (out, kernel)) in
            self.convp_out.as_chunks_mut::<NB_DF>().0.iter_mut().zip(self.convp.weight.chunks_exact(channels * CONVP_FRAMES)).enumerate()
        {
            out.fill(0.);
            let group = o / (COEFS / 2);
            for (c, taps) in kernel.as_chunks::<CONVP_FRAMES>().0.iter().enumerate() {
                let channel = group * channels + c;
                for (frame, &tap) in self.c0.frames().zip(taps) {
                    axpy(tap, &frame[channel * NB_DF..(channel + 1) * NB_DF], out);
                }
            }
        }
        self.convp_pointwise.pointwise(&self.convp_out, &mut self.convp_mixed, NB_DF);
        relu(&mut self.convp_mixed);
        for (o, mixed) in self.convp_mixed.as_chunks::<NB_DF>().0.iter().enumerate() {
            for (bin, &mixed) in mixed.iter().enumerate() {
                self.coefs[bin * COEFS + o] += mixed;
            }
        }
        &self.coefs
    }

    pub(crate) fn reset(&mut self) {
        self.c0.reset();
        self.gru0.reset();
        self.gru1.reset();
    }
}
