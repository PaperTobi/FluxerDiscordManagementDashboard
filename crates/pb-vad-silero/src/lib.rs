//! Silero VAD v6.2 (MIT, snakers4/silero-vad) written in Burn. The weights come straight from the official
//! `silero_vad.onnx` of silero-vad 6.2.3 (the file the old bot ran), read from its 16 kHz branch; the network follows the
//! project's own `tinygrad_model.py`. (The `silero_vad_16k.safetensors` published at the same tag holds different
//! weights and does not reproduce the ONNX model; see docs/proposals/0002.) Golden tests check the probabilities against
//! the ONNX model frame by frame. All streams' frames go through the network as one batch.

use std::path::Path;

use burn::module::{Module, Param};
use burn::nn::PaddingConfig1d;
use burn::nn::conv::{Conv1d, Conv1dConfig};
use burn::tensor::activation::{relu, sigmoid};
use burn::tensor::backend::Backend;
use burn::tensor::ops::PadMode;
use burn::tensor::{Tensor, TensorData};
use onnx_ir::ModelProto;
use pb_models_api::{FRAME, ModelError, VadInfo, VadModel, VadState, energy_gate};
use protobuf::Message;

/// Burn's pure-Rust CPU backend (the VAD's batches are tiny; a GPU would only add latency).
pub type Cpu = burn::backend::Flex;

/// The model file name in the model directory (silero-vad 6.2.3, sha256 1a153a22…).
pub const MODEL_FILE: &str = "silero_vad.onnx";
/// Samples of the previous frame carried into the next one.
pub const CONTEXT: usize = 64;
const N_FFT: usize = 256;
const HOP: usize = 128;
const HIDDEN: usize = 128;

/// `nn.LSTMCell(128, 128)` with PyTorch's parameter layout (gates in the order input, forget, cell, output).
#[derive(Module, Debug)]
pub struct LstmCell<B: Backend> {
    pub weight_ih: Param<Tensor<B, 2>>,
    pub weight_hh: Param<Tensor<B, 2>>,
    pub bias_ih: Param<Tensor<B, 1>>,
    pub bias_hh: Param<Tensor<B, 1>>,
}

impl<B: Backend> LstmCell<B> {
    /// `(h, c)` → `(h', c')`.
    fn forward(&self, x: Tensor<B, 2>, h: Tensor<B, 2>, c: Tensor<B, 2>) -> (Tensor<B, 2>, Tensor<B, 2>) {
        let gates = x.matmul(self.weight_ih.val().transpose())
            + self.bias_ih.val().unsqueeze_dim(0)
            + h.matmul(self.weight_hh.val().transpose())
            + self.bias_hh.val().unsqueeze_dim(0);
        let gate = |i: usize| gates.clone().narrow(1, i * HIDDEN, HIDDEN);
        let (i, f, g, o) = (sigmoid(gate(0)), sigmoid(gate(1)), gate(2).tanh(), sigmoid(gate(3)));
        let c = f * c + i * g;
        let h = o * c.clone().tanh();
        (h, c)
    }
}

/// The 16 kHz Silero network.
#[derive(Module, Debug)]
pub struct Silero<B: Backend> {
    pub stft_conv: Conv1d<B>,
    pub conv1: Conv1d<B>,
    pub conv2: Conv1d<B>,
    pub conv3: Conv1d<B>,
    pub conv4: Conv1d<B>,
    pub lstm_cell: LstmCell<B>,
    pub final_conv: Conv1d<B>,
}

fn param<B: Backend, const D: usize>(shape: [usize; D], device: &B::Device) -> Param<Tensor<B, D>> {
    Param::from_tensor(Tensor::zeros(shape, device))
}

impl<B: Backend> Silero<B> {
    fn new(device: &B::Device) -> Self {
        let conv = |i, o, stride| {
            Conv1dConfig::new(i, o, 3)
                .with_stride(stride)
                .with_padding(PaddingConfig1d::Explicit(1, 1))
                .init(device)
        };
        Silero {
            stft_conv: Conv1dConfig::new(1, 2 * (N_FFT / 2 + 1), N_FFT)
                .with_stride(HOP)
                .with_bias(false)
                .init(device),
            conv1: conv(N_FFT / 2 + 1, 128, 1),
            conv2: conv(128, 64, 2),
            conv3: conv(64, 64, 2),
            conv4: conv(64, 128, 1),
            lstm_cell: LstmCell {
                weight_ih: param([4 * HIDDEN, HIDDEN], device),
                weight_hh: param([4 * HIDDEN, HIDDEN], device),
                bias_ih: param([4 * HIDDEN], device),
                bias_hh: param([4 * HIDDEN], device),
            },
            final_conv: Conv1dConfig::new(HIDDEN, 1, 1).init(device),
        }
    }

    /// `x`: `[B, CONTEXT + FRAME]`; returns speech probabilities `[B]` and the new `(h, c)`.
    fn forward(&self, x: Tensor<B, 2>, h: Tensor<B, 2>, c: Tensor<B, 2>) -> (Tensor<B, 1>, Tensor<B, 2>, Tensor<B, 2>) {
        let [b, _] = x.dims();
        let x = x.pad([(0, 0), (0, CONTEXT)], PadMode::Reflect).unsqueeze_dim::<3>(1);
        let spec = self.stft_conv.forward(x);
        let cutoff = N_FFT / 2 + 1;
        let re = spec.clone().narrow(1, 0, cutoff);
        let im = spec.narrow(1, cutoff, cutoff);
        let mag = (re.clone() * re + im.clone() * im).sqrt();
        let x = relu(self.conv1.forward(mag));
        let x = relu(self.conv2.forward(x));
        let x = relu(self.conv3.forward(x));
        let x = relu(self.conv4.forward(x));
        let [_, channels, steps] = x.dims();
        debug_assert_eq!(steps, 1, "the 576-sample input reduces to one step");
        let x = x.reshape([b, channels]);
        let (h, c) = self.lstm_cell.forward(x, h, c);
        let out = sigmoid(self.final_conv.forward(relu(h.clone()).unsqueeze_dim(2)));
        (out.reshape([b]), h, c)
    }
}

/// Silero VAD on the CPU.
pub struct SileroVad {
    model: Silero<Cpu>,
    info: VadInfo,
    device: burn_flex::FlexDevice,
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
        let device = Default::default();
        let mut w = onnx_weights(&dir.join(MODEL_FILE))?;
        let mut take = |name: &str, shape: &[usize]| -> Result<TensorData, ModelError> {
            let (dims, values) = w
                .remove(name)
                .ok_or_else(|| ModelError::Load(format!("{MODEL_FILE} has no {name}")))?;
            if dims != shape {
                return Err(ModelError::Load(format!(
                    "{name} has shape {dims:?}, expected {shape:?}"
                )));
            }
            Ok(TensorData::new(values, dims))
        };
        let mut model = Silero::<Cpu>::new(&device);
        let bins = N_FFT / 2 + 1;
        model.stft_conv.weight = tensor(&device, take("stft.forward_basis_buffer", &[2 * bins, 1, N_FFT])?);
        let encoder: [(&mut Conv1d<Cpu>, [usize; 3]); 4] = [
            (&mut model.conv1, [128, bins, 3]),
            (&mut model.conv2, [64, 128, 3]),
            (&mut model.conv3, [64, 64, 3]),
            (&mut model.conv4, [128, 64, 3]),
        ];
        for (i, (conv, shape)) in encoder.into_iter().enumerate() {
            conv.weight = tensor(&device, take(&format!("encoder.{i}.reparam_conv.weight"), &shape)?);
            conv.bias = Some(tensor(
                &device,
                take(&format!("encoder.{i}.reparam_conv.bias"), &shape[..1])?,
            ));
        }
        model.lstm_cell.weight_ih = tensor(&device, take("decoder.rnn.weight_ih", &[4 * HIDDEN, HIDDEN])?);
        model.lstm_cell.weight_hh = tensor(&device, take("decoder.rnn.weight_hh", &[4 * HIDDEN, HIDDEN])?);
        model.lstm_cell.bias_ih = tensor(&device, take("decoder.rnn.bias_ih", &[4 * HIDDEN])?);
        model.lstm_cell.bias_hh = tensor(&device, take("decoder.rnn.bias_hh", &[4 * HIDDEN])?);
        model.final_conv.weight = tensor(&device, take("decoder.decoder.2.weight", &[1, HIDDEN, 1])?);
        model.final_conv.bias = Some(tensor(&device, take("decoder.decoder.2.bias", &[1])?));
        let info = VadInfo {
            model: "silero-vad 6.2.3 (16 kHz)".into(),
            context: CONTEXT,
        };
        Ok(SileroVad {
            model: model.no_grad(),
            info,
            device,
        })
    }
}

fn tensor<const D: usize>(device: &burn_flex::FlexDevice, data: TensorData) -> Param<Tensor<Cpu, D>> {
    Param::from_tensor(Tensor::from_data(data, device))
}

/// The named float tensors of the 16 kHz branch of `silero_vad.onnx`. The graph is `If(sr == C, then, else)`; the
/// 16 kHz branch is whichever one `C` selects for 16000.
/// Named tensors: shape and values.
type Weights = std::collections::HashMap<String, (Vec<usize>, Vec<f32>)>;

fn onnx_weights(path: &Path) -> Result<Weights, ModelError> {
    let load = |e: String| ModelError::Load(format!("{}: {e}", path.display()));
    let bytes = std::fs::read(path).map_err(|e| load(e.to_string()))?;
    let model = ModelProto::parse_from_bytes(&bytes).map_err(|e| load(e.to_string()))?;
    let graph = &model.graph;
    let constant = graph
        .node
        .iter()
        .find(|n| n.op_type == "Constant")
        .and_then(|n| n.attribute.iter().find(|a| a.name == "value"))
        .map(|a| int_value(&a.t))
        .ok_or_else(|| load("no sample-rate constant".into()))?;
    let branch_name = if constant == Some(16_000) {
        "then_branch"
    } else {
        "else_branch"
    };
    let branch = graph
        .node
        .iter()
        .find(|n| n.op_type == "If")
        .and_then(|n| n.attribute.iter().find(|a| a.name == branch_name))
        .map(|a| &a.g)
        .ok_or_else(|| load(format!("no {branch_name}")))?;
    let prefix = format!("If_0_{branch_name}__Inline_0__");
    let mut out = std::collections::HashMap::new();
    for node in &branch.node {
        let Some(name) = node.output.first().and_then(|o| o.strip_prefix(&prefix)) else {
            continue;
        };
        if node.op_type != "Constant" || !name.contains('.') || name.starts_with("self.") {
            continue;
        }
        let Some(attr) = node.attribute.iter().find(|a| a.name == "value") else {
            continue;
        };
        let t = &attr.t;
        if t.data_type != 1 {
            continue; // not FLOAT
        }
        let dims: Vec<usize> = t.dims.iter().map(|d| usize::try_from(*d).unwrap_or(0)).collect();
        let values: Vec<f32> = if t.float_data.is_empty() {
            t.raw_data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        } else {
            t.float_data.clone()
        };
        if values.len() != dims.iter().product::<usize>() {
            return Err(load(format!("{name}: {} values for shape {dims:?}", values.len())));
        }
        out.insert(name.to_owned(), (dims, values));
    }
    Ok(out)
}

fn int_value(t: &onnx_ir::TensorProto) -> Option<i64> {
    t.int64_data.first().copied().or_else(|| {
        (t.raw_data.len() >= 8).then(|| {
            let b = &t.raw_data[..8];
            i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        })
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
        let b = frames.len();
        if b == 0 {
            return Vec::new();
        }
        let mut input = Vec::with_capacity(b * (CONTEXT + FRAME));
        let mut h = Vec::with_capacity(b * HIDDEN);
        let mut c = Vec::with_capacity(b * HIDDEN);
        for (frame, state) in frames.iter().zip(states.iter()) {
            assert_eq!(state.0.len(), STATE_LEN, "a state made by this model");
            h.extend_from_slice(&state.0[..HIDDEN]);
            c.extend_from_slice(&state.0[HIDDEN..2 * HIDDEN]);
            input.extend_from_slice(&state.0[2 * HIDDEN..]);
            input.extend_from_slice(frame);
        }
        let d = &self.device;
        let x = Tensor::<Cpu, 2>::from_data(TensorData::new(input, [b, CONTEXT + FRAME]), d);
        let h = Tensor::<Cpu, 2>::from_data(TensorData::new(h, [b, HIDDEN]), d);
        let c = Tensor::<Cpu, 2>::from_data(TensorData::new(c, [b, HIDDEN]), d);
        let (p, h, c) = self.model.forward(x, h, c);
        let p: Vec<f32> = p.into_data().to_vec().unwrap_or_default();
        let h: Vec<f32> = h.into_data().to_vec().unwrap_or_default();
        let c: Vec<f32> = c.into_data().to_vec().unwrap_or_default();
        for (i, (frame, state)) in frames.iter().zip(states.iter_mut()).enumerate() {
            state.0[..HIDDEN].copy_from_slice(&h[i * HIDDEN..(i + 1) * HIDDEN]);
            state.0[HIDDEN..2 * HIDDEN].copy_from_slice(&c[i * HIDDEN..(i + 1) * HIDDEN]);
            state.0[2 * HIDDEN..].copy_from_slice(&frame[FRAME - CONTEXT..]);
        }
        p
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
