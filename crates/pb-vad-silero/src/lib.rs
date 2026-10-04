//! Silero VAD v6.2 (MIT, snakers4/silero-vad) as a hand-written forward pass. The weights come straight from the
//! official `silero_vad.onnx` of silero-vad 6.2.3 (the file the old bot ran), read from its 16 kHz branch; the network
//! follows the project's own `tinygrad_model.py`. (The `silero_vad_16k.safetensors` published at the same tag holds
//! different weights and does not reproduce the ONNX model; see docs/proposals/0002.) Golden tests check the
//! probabilities against the ONNX model frame by frame. All streams' frames go through the network as one batch.
//!
//! The network is small (about 0.7 million multiply-adds per frame), so a tensor library spends most of a step
//! dispatching its few dozen operations. Here each layer is one pass over weights laid out for it at load time, in the
//! widest vector instructions the CPU has (chosen when it runs), and the buffers are kept from step to step.

use std::ops::Range;
use std::path::Path;

use pb_models_api::{FRAME, ModelError, VadInfo, VadModel, VadState, energy_gate};
use rten_onnx::onnx::{AttributeProto, DataType, ModelProto, NodeProto, TensorProto};
use rten_simd::ops::{BitOps, NumOps};
use rten_simd::{Isa, SimdOp};

/// The model file name in the model directory (silero-vad 6.2.3, sha256 1a153a22…).
pub const MODEL_FILE: &str = "silero_vad.onnx";
/// Samples of the previous frame carried into the next one.
pub const CONTEXT: usize = 64;
const N_FFT: usize = 256;
const HOP: usize = 128;
/// Frequency bins of the STFT.
const BINS: usize = N_FFT / 2 + 1;
const HIDDEN: usize = 128;
/// Samples per stream going into the STFT: the context, the frame, and the frame's end mirrored.
const PADDED: usize = CONTEXT + FRAME + CONTEXT;
/// STFT windows per frame.
const WINDOWS: usize = (PADDED - N_FFT) / HOP + 1;

/// Input rows that share one pass over a layer's weights.
const ROWS: usize = 4;

/// The width of the CPU's `f32` vectors as [`rten_simd`] dispatches them: 16 with AVX-512, 8 with AVX2, 4 otherwise.
struct VectorLen;

impl SimdOp for VectorLen {
    type Output = usize;

    #[inline(always)]
    fn eval<I: Isa>(self, isa: I) -> usize {
        isa.f32().len()
    }
}

/// A dense layer `y = bias + x · W` (`W`: inputs × outputs). `W` is stored in panels of two vectors' worth of outputs
/// (`lanes`), each holding its outputs' weights input after input, so the innermost loop reads memory front to back.
/// The last panel is filled up with zero weights and biases.
#[derive(Debug)]
struct Dense {
    inputs: usize,
    lanes: usize,
    panels: Vec<f32>,
    bias: Vec<f32>,
}

impl Dense {
    /// `weight(i, o)` is the weight from input `i` to output `o`.
    fn new(inputs: usize, outputs: usize, lanes: usize, bias: &[f32], weight: impl Fn(usize, usize) -> f32) -> Self {
        let width = outputs.div_ceil(lanes) * lanes;
        let mut panels = vec![0.0; inputs * width];
        for (p, panel) in panels.chunks_exact_mut(inputs * lanes).enumerate() {
            for (i, row) in panel.chunks_exact_mut(lanes).enumerate() {
                for (l, w) in row.iter_mut().enumerate() {
                    let o = p * lanes + l;
                    if o < outputs {
                        *w = weight(i, o);
                    }
                }
            }
        }
        let mut bias = bias.to_vec();
        bias.resize(width, 0.0);
        Dense {
            inputs,
            lanes,
            panels,
            bias,
        }
    }

    /// Values per output row (the outputs rounded up to whole panels).
    fn width(&self) -> usize {
        self.bias.len()
    }

    /// For each row `r < rows`: `y[out(r)..][..width] = bias + Σ x[at(r) + j] · W[inputs.start + j]`, summed over
    /// `j < inputs.len()` (a layer may skip inputs that are zero in every row), and ReLU'd if `relu`.
    #[allow(clippy::too_many_arguments)] // two (offset, …) pairs and the input range: grouping them would add types, not clarity
    #[inline(always)] // see `Forward`
    fn apply<I: Isa>(
        &self,
        isa: I,
        x: &[f32],
        at: impl Fn(usize) -> usize,
        inputs: Range<usize>,
        rows: usize,
        y: &mut [f32],
        out: impl Fn(usize) -> usize,
        relu: bool,
    ) {
        assert_eq!(self.lanes, 2 * isa.f32().len(), "weights packed for this CPU's vectors");
        let (lanes, len) = (self.lanes, inputs.len());
        for (p, panel) in self.panels.chunks_exact(self.inputs * lanes).enumerate() {
            let k = Kernel {
                w: &panel[inputs.start * lanes..inputs.end * lanes],
                bias: &self.bias[p * lanes..(p + 1) * lanes],
                len,
                relu,
            };
            let col = p * lanes;
            let mut r = 0;
            while r < rows {
                let n = (rows - r).min(ROWS);
                let x_at = |m: usize| at(r + m);
                let y_at = |m: usize| out(r + m) + col;
                // Fewer rows keep fewer sums in flight; splitting the inputs into interleaved partial sums keeps the
                // multiply-add units busy all the same.
                match n {
                    1 => k.rows::<I, 1, 4>(isa, x, x_at, y, y_at),
                    2 => k.rows::<I, 2, 2>(isa, x, x_at, y, y_at),
                    3 => k.rows::<I, 3, 1>(isa, x, x_at, y, y_at),
                    _ => k.rows::<I, ROWS, 1>(isa, x, x_at, y, y_at),
                }
                r += n;
            }
        }
    }
}

/// One panel of a [`Dense`] layer, applied to a few rows at a time.
struct Kernel<'a> {
    /// The panel's weights for the inputs in use.
    w: &'a [f32],
    bias: &'a [f32],
    /// Inputs per row.
    len: usize,
    relu: bool,
}

impl Kernel<'_> {
    /// `M` rows (starting at `x_at(m)`, written at `y_at(m)`), with `P` partial sums each, all held in registers while
    /// the panel streams past once.
    #[inline(always)]
    fn rows<I: Isa, const M: usize, const P: usize>(
        &self,
        isa: I,
        x: &[f32],
        x_at: impl Fn(usize) -> usize,
        y: &mut [f32],
        y_at: impl Fn(usize) -> usize,
    ) {
        let ops = isa.f32();
        let lanes = 2 * ops.len();
        let xs: [&[f32]; M] = std::array::from_fn(|m| &x[x_at(m)..x_at(m) + self.len]);
        let mut acc = [[[ops.zero(); 2]; M]; P];
        acc[0] = [ops.load_many::<2>(self.bias); M];
        let (whole, rest) = self.w.split_at(self.len / P * P * lanes);
        for (jp, wp) in whole.chunks_exact(P * lanes).enumerate() {
            for (p, (acc, w)) in acc.iter_mut().zip(wp.chunks_exact(lanes)).enumerate() {
                let w = ops.load_many::<2>(w);
                for (acc, x) in acc.iter_mut().zip(&xs) {
                    let x = ops.splat(x[jp * P + p]);
                    acc[0] = ops.mul_add(x, w[0], acc[0]);
                    acc[1] = ops.mul_add(x, w[1], acc[1]);
                }
            }
        }
        let first = self.len / P * P;
        for (j, w) in rest.chunks_exact(lanes).enumerate() {
            let w = ops.load_many::<2>(w);
            for (acc, x) in acc[0].iter_mut().zip(&xs) {
                let x = ops.splat(x[first + j]);
                acc[0] = ops.mul_add(x, w[0], acc[0]);
                acc[1] = ops.mul_add(x, w[1], acc[1]);
            }
        }
        for m in 0..M {
            let dst = &mut y[y_at(m)..y_at(m) + lanes];
            for (v, dst) in dst.chunks_exact_mut(ops.len()).enumerate() {
                let mut sum = acc[0][m][v];
                for partial in &acc[1..] {
                    sum = ops.add(sum, partial[m][v]);
                }
                if self.relu {
                    sum = ops.max(sum, ops.zero());
                }
                ops.store(sum, dst);
            }
        }
    }
}

/// A 1-D convolution with kernel 3 and padding 1, then ReLU: a [`Dense`] layer over (tap, input channel) pairs.
#[derive(Debug)]
struct Conv {
    dense: Dense,
    channels: usize,
    stride: usize,
}

impl Conv {
    /// `weight`: the ONNX layout `[outputs][channels][3]`.
    fn new(weight: &[f32], bias: &[f32], channels: usize, stride: usize, lanes: usize) -> Self {
        let dense = Dense::new(3 * channels, bias.len(), lanes, bias, |i, o| {
            weight[(o * channels + i % channels) * 3 + i / channels]
        });
        Conv {
            dense,
            channels,
            stride,
        }
    }

    /// Output steps for `steps` input steps.
    fn steps(&self, steps: usize) -> usize {
        (steps - 1) / self.stride + 1
    }

    /// `x` holds every stream's `steps` input steps between two zero steps (the padding). Output step `t` of stream
    /// `s` goes to `y[s · y_stream + y_first + t · width..]`.
    #[allow(clippy::too_many_arguments)] // the output's layout is three numbers
    #[inline(always)] // see `Forward`
    fn forward<I: Isa>(
        &self,
        isa: I,
        x: &[f32],
        streams: usize,
        steps: usize,
        y: &mut [f32],
        y_stream: usize,
        y_first: usize,
    ) {
        let (c, s, out_steps, width) = (self.channels, self.stride, self.steps(steps), self.dense.width());
        // Output step t's tap k reads padded step t·s + k. Taps that read only the zero steps, for every t, are skipped.
        let first = (0..out_steps).map(|t| 1usize.saturating_sub(t * s)).min().unwrap_or(0);
        let last = (0..out_steps).map(|t| (steps - t * s).min(2)).max().unwrap_or(2);
        self.dense.apply(
            isa,
            x,
            |r| ((r / out_steps) * (steps + 2) + (r % out_steps) * s + first) * c,
            first * c..(last + 1) * c,
            streams * out_steps,
            y,
            |r| (r / out_steps) * y_stream + y_first + (r % out_steps) * width,
            true,
        );
    }
}

/// The 16 kHz Silero network.
#[derive(Debug)]
struct Silero {
    /// The STFT as a layer: one window's samples → real parts ‖ imaginary parts.
    stft: Dense,
    encoder: [Conv; 4],
    /// `nn.LSTMCell(128, 128)`: input ‖ hidden state → the gates input, forget, cell, output (both biases summed).
    lstm: Dense,
    /// The decoder's 1×1 convolution.
    head: Vec<f32>,
    head_bias: f32,
}

/// Buffers kept between steps. They only grow (when a step has more streams than any before), and the zero steps
/// around each stream's encoder activations are never written.
#[derive(Debug, Default)]
struct Scratch {
    /// Per stream: context ‖ frame ‖ mirrored end (`PADDED`).
    input: Vec<f32>,
    /// Per window: the STFT's real ‖ imaginary parts.
    spec: Vec<f32>,
    /// The inputs of the four convolutions, per stream with a zero step on either side.
    acts: [Vec<f32>; 4],
    /// Per stream: the encoder's output ‖ the hidden state.
    cell_in: Vec<f32>,
    /// Per stream: the four gates.
    gates: Vec<f32>,
}

fn fit(buf: &mut Vec<f32>, len: usize) {
    if buf.len() < len {
        buf.resize(len, 0.0);
    }
}

impl Silero {
    /// From `scratch.input` and `scratch.cell_in`'s hidden states to `scratch.gates`, for `streams` streams.
    #[inline(always)] // see `Forward`
    fn gates<I: Isa>(&self, isa: I, scratch: &mut Scratch, streams: usize) {
        let s = scratch;
        let width = self.stft.width();
        fit(&mut s.spec, streams * WINDOWS * width);
        self.stft.apply(
            isa,
            &s.input,
            |r| (r / WINDOWS) * PADDED + (r % WINDOWS) * HOP,
            0..N_FFT,
            streams * WINDOWS,
            &mut s.spec,
            |r| r * width,
            false,
        );
        let mut steps = WINDOWS;
        fit(&mut s.acts[0], streams * (steps + 2) * BINS);
        for (r, spec) in s.spec.chunks_exact(width).take(streams * WINDOWS).enumerate() {
            let at = ((r / WINDOWS) * (steps + 2) + r % WINDOWS + 1) * BINS;
            let (re, im) = spec[..2 * BINS].split_at(BINS);
            for ((m, re), im) in s.acts[0][at..at + BINS].iter_mut().zip(re).zip(im) {
                *m = (re * re + im * im).sqrt();
            }
        }
        for (i, conv) in self.encoder.iter().enumerate() {
            let next = conv.steps(steps);
            let width = conv.dense.width();
            let (x, rest) = s.acts.split_at_mut(i + 1);
            let x = &x[i];
            if let Some(y) = rest.first_mut() {
                fit(y, streams * (next + 2) * width);
                conv.forward(isa, x, streams, steps, y, (next + 2) * width, width);
            } else {
                debug_assert_eq!(
                    (next, width),
                    (1, HIDDEN),
                    "the encoder ends in one step of HIDDEN channels"
                );
                conv.forward(isa, x, streams, steps, &mut s.cell_in, 2 * HIDDEN, 0);
            }
            steps = next;
        }
        fit(&mut s.gates, streams * 4 * HIDDEN);
        self.lstm.apply(
            isa,
            &s.cell_in,
            |r| r * 2 * HIDDEN,
            0..2 * HIDDEN,
            streams,
            &mut s.gates,
            |r| r * 4 * HIDDEN,
            false,
        );
    }
}

/// [`Silero::gates`] compiled for the widest vector instructions the CPU has (AVX-512, AVX2 with FMA, or the
/// baseline), chosen when it runs. Everything it calls is `#[inline(always)]` so that it is compiled inside the
/// dispatched function, with those instructions enabled.
struct Forward<'a> {
    net: &'a Silero,
    scratch: &'a mut Scratch,
    streams: usize,
}

impl SimdOp for Forward<'_> {
    type Output = ();

    #[inline(always)]
    fn eval<I: Isa>(self, isa: I) {
        self.net.gates(isa, self.scratch, self.streams);
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Silero VAD on the CPU.
pub struct SileroVad {
    net: Silero,
    scratch: Scratch,
    info: VadInfo,
}

impl std::fmt::Debug for SileroVad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SileroVad")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl SileroVad {
    /// Loads `silero_vad.onnx` from `dir`.
    pub fn load(dir: &Path) -> Result<Self, ModelError> {
        let mut w = onnx_weights(&dir.join(MODEL_FILE))?;
        let mut take = |name: &str, shape: &[usize]| -> Result<Vec<f32>, ModelError> {
            let (dims, values) = w
                .remove(name)
                .ok_or_else(|| ModelError::Load(format!("{MODEL_FILE} has no {name}")))?;
            if dims != shape {
                return Err(ModelError::Load(format!(
                    "{name} has shape {dims:?}, expected {shape:?}"
                )));
            }
            Ok(values)
        };
        let lanes = 2 * VectorLen.dispatch();
        let basis = take("stft.forward_basis_buffer", &[2 * BINS, 1, N_FFT])?;
        let stft = Dense::new(N_FFT, 2 * BINS, lanes, &[], |i, o| basis[o * N_FFT + i]);
        let mut conv = |i: usize, outputs: usize, channels: usize, stride: usize| -> Result<Conv, ModelError> {
            let weight = take(&format!("encoder.{i}.reparam_conv.weight"), &[outputs, channels, 3])?;
            let bias = take(&format!("encoder.{i}.reparam_conv.bias"), &[outputs])?;
            Ok(Conv::new(&weight, &bias, channels, stride, lanes))
        };
        let encoder = [
            conv(0, 128, BINS, 1)?,
            conv(1, 64, 128, 2)?,
            conv(2, 64, 64, 2)?,
            conv(3, HIDDEN, 64, 1)?,
        ];
        let w_ih = take("decoder.rnn.weight_ih", &[4 * HIDDEN, HIDDEN])?;
        let w_hh = take("decoder.rnn.weight_hh", &[4 * HIDDEN, HIDDEN])?;
        let b_ih = take("decoder.rnn.bias_ih", &[4 * HIDDEN])?;
        let b_hh = take("decoder.rnn.bias_hh", &[4 * HIDDEN])?;
        let bias: Vec<f32> = b_ih.iter().zip(&b_hh).map(|(a, b)| a + b).collect();
        let lstm = Dense::new(2 * HIDDEN, 4 * HIDDEN, lanes, &bias, |i, o| {
            if i < HIDDEN {
                w_ih[o * HIDDEN + i]
            } else {
                w_hh[o * HIDDEN + i - HIDDEN]
            }
        });
        let head = take("decoder.decoder.2.weight", &[1, HIDDEN, 1])?;
        let head_bias = take("decoder.decoder.2.bias", &[1])?[0];
        Ok(SileroVad {
            net: Silero {
                stft,
                encoder,
                lstm,
                head,
                head_bias,
            },
            scratch: Scratch::default(),
            info: VadInfo {
                model: "silero-vad 6.2.3 (16 kHz)".into(),
                context: CONTEXT,
            },
        })
    }
}

/// Named tensors: shape and values.
type Weights = std::collections::HashMap<String, (Vec<usize>, Vec<f32>)>;

/// The named float tensors of the 16 kHz branch of `silero_vad.onnx`. The graph is `If(sr == C, then, else)`; the
/// 16 kHz branch is whichever one `C` selects for 16000.
fn onnx_weights(path: &Path) -> Result<Weights, ModelError> {
    let load = |e: String| ModelError::Load(format!("{}: {e}", path.display()));
    let file = std::fs::File::open(path).map_err(|e| load(e.to_string()))?;
    let model = ModelProto::parse_file(file).map_err(|e| load(e.to_string()))?;
    let nodes = model.graph.as_ref().map_or(&[][..], |g| &g.node[..]);
    let constant = nodes
        .iter()
        .find(|n| n.op_type.as_deref() == Some("Constant"))
        .and_then(|n| attribute(n, "value"))
        .and_then(|a| a.t.as_ref())
        .map(int_value)
        .ok_or_else(|| load("no sample-rate constant".into()))?;
    let branch_name = if constant == Some(16_000) {
        "then_branch"
    } else {
        "else_branch"
    };
    let branch = nodes
        .iter()
        .find(|n| n.op_type.as_deref() == Some("If"))
        .and_then(|n| attribute(n, branch_name))
        .and_then(|a| a.g.as_ref())
        .ok_or_else(|| load(format!("no {branch_name}")))?;
    let prefix = format!("If_0_{branch_name}__Inline_0__");
    let mut out = std::collections::HashMap::new();
    for node in &branch.node {
        let Some(name) = node.output.first().and_then(|o| o.strip_prefix(&prefix)) else {
            continue;
        };
        if node.op_type.as_deref() != Some("Constant") || !name.contains('.') || name.starts_with("self.") {
            continue;
        }
        let Some(t) = attribute(node, "value").and_then(|a| a.t.as_ref()) else {
            continue;
        };
        if t.data_type != Some(DataType::FLOAT) {
            continue;
        }
        let dims: Vec<usize> = t.dims.iter().map(|d| usize::try_from(*d).unwrap_or(0)).collect();
        let values: Vec<f32> = match &t.raw_data {
            Some(raw) if t.float_data.is_empty() => raw
                .borrow()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect(),
            _ => t.float_data.clone(),
        };
        if values.len() != dims.iter().product::<usize>() {
            return Err(load(format!("{name}: {} values for shape {dims:?}", values.len())));
        }
        out.insert(name.to_owned(), (dims, values));
    }
    Ok(out)
}

fn attribute<'a>(node: &'a NodeProto, name: &str) -> Option<&'a AttributeProto> {
    node.attribute.iter().find(|a| a.name.as_deref() == Some(name))
}

fn int_value(t: &TensorProto) -> Option<i64> {
    t.int64_data.first().copied().or_else(|| {
        let raw = t.raw_data.as_ref()?.borrow();
        let b: [u8; 8] = raw.get(..8)?.try_into().ok()?;
        Some(i64::from_le_bytes(b))
    })
}

/// State layout: `h[128] ‖ c[128] ‖ context[64]`.
const STATE_LEN: usize = 2 * HIDDEN + CONTEXT;

impl VadModel for SileroVad {
    fn info(&self) -> &VadInfo {
        &self.info
    }

    fn new_state(&self) -> VadState {
        VadState(vec![0.0; STATE_LEN])
    }

    fn step(&mut self, frames: &[[f32; FRAME]], states: &mut [&mut VadState]) -> Vec<f32> {
        assert_eq!(frames.len(), states.len(), "one state per frame");
        let streams = frames.len();
        if streams == 0 {
            return Vec::new();
        }
        let s = &mut self.scratch;
        fit(&mut s.input, streams * PADDED);
        fit(&mut s.cell_in, streams * 2 * HIDDEN);
        let inputs = s.input.as_chunks_mut::<PADDED>().0.iter_mut();
        let cells = s.cell_in.as_chunks_mut::<{ 2 * HIDDEN }>().0.iter_mut();
        let inputs = inputs.zip(cells);
        for ((frame, state), (input, cell_in)) in frames.iter().zip(states.iter()).zip(inputs) {
            assert_eq!(state.0.len(), STATE_LEN, "a state made by this model");
            let (context, rest) = input.split_at_mut(CONTEXT);
            let (body, mirror) = rest.split_at_mut(FRAME);
            context.copy_from_slice(&state.0[2 * HIDDEN..]);
            body.copy_from_slice(frame);
            // Reflection padding: the frame's end mirrored, without repeating its last sample.
            for (m, x) in mirror.iter_mut().zip(frame[..FRAME - 1].iter().rev()) {
                *m = *x;
            }
            cell_in[HIDDEN..].copy_from_slice(&state.0[..HIDDEN]);
        }
        let net = &self.net;
        Forward {
            net,
            scratch: s,
            streams,
        }
        .dispatch();
        let gates = s.gates.as_chunks::<{ 4 * HIDDEN }>().0;
        frames
            .iter()
            .zip(states.iter_mut())
            .zip(gates)
            .map(|((frame, state), gates)| {
                let (h, rest) = state.0.split_at_mut(HIDDEN);
                let (c, context) = rest.split_at_mut(HIDDEN);
                let (i, rest) = gates.split_at(HIDDEN);
                let (f, rest) = rest.split_at(HIDDEN);
                let (g, o) = rest.split_at(HIDDEN);
                let mut logit = net.head_bias;
                for u in 0..HIDDEN {
                    c[u] = sigmoid(f[u]) * c[u] + sigmoid(i[u]) * g[u].tanh();
                    h[u] = sigmoid(o[u]) * c[u].tanh();
                    logit += h[u].max(0.0) * net.head[u];
                }
                context.copy_from_slice(&frame[FRAME - CONTEXT..]);
                sigmoid(logit)
            })
            .collect()
    }
}

/// The fallback when Silero cannot be used: an adaptive noise-floor gate (speech = clearly above the quietest recent
/// level). An exact port of the old bot's `EnergyVad`; its state is the floor in dBFS (NaN = none yet).
#[derive(Debug, Clone)]
pub struct EnergyVad {
    info: VadInfo,
}

impl Default for EnergyVad {
    fn default() -> Self {
        EnergyVad {
            info: VadInfo {
                model: "energy gate".into(),
                context: 0,
            },
        }
    }
}

impl VadModel for EnergyVad {
    fn info(&self) -> &VadInfo {
        &self.info
    }

    fn new_state(&self) -> VadState {
        VadState(vec![f32::NAN])
    }

    fn step(&mut self, frames: &[[f32; FRAME]], states: &mut [&mut VadState]) -> Vec<f32> {
        frames
            .iter()
            .zip(states.iter_mut())
            .map(|(f, s)| {
                if s.0.is_empty() {
                    s.0.push(f32::NAN);
                }
                energy_gate(&mut s.0[0], f)
            })
            .collect()
    }
}

#[cfg(test)]
mod dense_tests {
    use super::*;

    /// `Dense::apply` against a plain matrix product, for 1 to 9 rows (every row-block shape), all inputs or a part,
    /// with and without ReLU.
    fn matches_a_plain_product<I: Isa>(isa: I) {
        let (inputs, outputs) = (37, 45);
        let weight = |i: usize, o: usize| ((i * 7 + o * 13) % 17) as f32 / 17.0 - 0.5;
        let bias: Vec<f32> = (0..outputs).map(|o| o as f32 / 50.0 - 0.3).collect();
        let dense = Dense::new(inputs, outputs, 2 * isa.f32().len(), &bias, weight);
        let width = dense.width();
        let stride = inputs + 3;
        for rows in 1..=9 {
            let x: Vec<f32> = (0..rows * stride)
                .map(|k| ((k * 31) % 23) as f32 / 23.0 - 0.4)
                .collect();
            for (used, relu) in [(0..inputs, false), (5..30, true)] {
                let mut y = vec![f32::NAN; rows * width];
                dense.apply(
                    isa,
                    &x,
                    |r| r * stride + 2,
                    used.clone(),
                    rows,
                    &mut y,
                    |r| r * width,
                    relu,
                );
                for r in 0..rows {
                    for o in 0..outputs {
                        let sum: f32 = used
                            .clone()
                            .enumerate()
                            .map(|(j, i)| x[r * stride + 2 + j] * weight(i, o))
                            .sum();
                        let want = if relu { (bias[o] + sum).max(0.0) } else { bias[o] + sum };
                        let got = y[r * width + o];
                        assert!(
                            (got - want).abs() < 1e-5,
                            "{rows} rows, row {r}, output {o}: {got} vs {want}"
                        );
                    }
                }
            }
        }
    }

    struct Dispatched;

    impl SimdOp for Dispatched {
        type Output = ();

        fn eval<I: Isa>(self, isa: I) {
            matches_a_plain_product(isa);
        }
    }

    #[test]
    fn dense_layers_compute_the_product_with_every_instruction_set() {
        matches_a_plain_product(rten_simd::isa::GenericIsa::new());
        #[cfg(target_arch = "x86_64")]
        if let Some(isa) = rten_simd::isa::Avx2Isa::new() {
            matches_a_plain_product(isa);
        }
        Dispatched.dispatch();
    }
}

#[cfg(test)]
mod energy_tests {
    use super::*;

    #[test]
    fn speech_above_the_floor_is_speech() {
        let mut vad = EnergyVad::default();
        let mut st = vad.new_state();
        let quiet = [0.001f32; FRAME];
        let loud: [f32; FRAME] = std::array::from_fn(|i| ((i as f32) * 0.3).sin() * 0.3);
        for _ in 0..20 {
            assert!(vad.step(&[quiet], &mut [&mut st])[0] < 0.1);
        }
        assert!(vad.step(&[loud], &mut [&mut st])[0] > 0.9);
    }
}
