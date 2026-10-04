//! The Roblox voice-safety classifier v3 as Burn modules, one per PyTorch module of `inference.py`, with the same
//! parameter names (see `crate::load` for the few renames).

use burn::module::{Module, Param};
use burn::nn::conv::{Conv1d, Conv1dConfig};
use burn::nn::{LayerNorm, Linear, LinearConfig, PaddingConfig1d};
use burn::tensor::activation::{gelu, sigmoid, silu, softmax};
use burn::tensor::backend::Backend;
use burn::tensor::{Tensor, TensorData};

use crate::config::ModelConfig;
use crate::frontend::{FeatureExtractor, Frontend};
use crate::mask;
use crate::ops::{Ops, Then, key_padding, layer_norm};

/// `nn.MultiheadAttention` (batch first, fused input projection) with a key-padding mask.
#[derive(Module, Debug)]
pub struct Mha<B: Backend> {
    pub in_proj: Linear<B>,
    pub out_proj: Linear<B>,
    pub heads: usize,
}

impl<B: Backend> Mha<B> {
    fn new(d: usize, heads: usize, device: &B::Device) -> Self {
        Mha {
            in_proj: LinearConfig::new(d, 3 * d).init(device),
            out_proj: LinearConfig::new(d, d).init(device),
            heads,
        }
    }
}

impl<B: Ops> Mha<B> {
    /// `residual + attention(x)`. `x`: `[1, L, d]`; `keys` from [`Ops::keys`].
    fn forward(&self, x: Tensor<B, 3>, keys: &B::Keys, residual: Tensor<B, 3>) -> Tensor<B, 3> {
        let qkv = B::linear_layer(x, &self.in_proj, Then::Keep);
        let out = B::self_attention(qkv, self.heads, keys);
        B::linear_layer(out, &self.out_proj, Then::Add(residual))
    }
}

/// `nn.TransformerEncoderLayer(norm_first=True, activation="gelu", batch_first=True)` in eval mode.
#[derive(Module, Debug)]
pub struct EncoderLayer<B: Backend> {
    pub self_attn: Mha<B>,
    pub linear1: Linear<B>,
    pub linear2: Linear<B>,
    pub norm1: LayerNorm<B>,
    pub norm2: LayerNorm<B>,
}

impl<B: Backend> EncoderLayer<B> {
    fn new(d: usize, heads: usize, device: &B::Device) -> Self {
        EncoderLayer {
            self_attn: Mha::new(d, heads, device),
            linear1: LinearConfig::new(d, 4 * d).init(device),
            linear2: LinearConfig::new(4 * d, d).init(device),
            norm1: layer_norm(d, device),
            norm2: layer_norm(d, device),
        }
    }
}

impl<B: Ops> EncoderLayer<B> {
    fn forward(&self, x: Tensor<B, 3>, keys: &B::Keys) -> Tensor<B, 3> {
        let x = self.self_attn.forward(B::normalize(x.clone(), &self.norm1), keys, x);
        let h = B::linear_layer(B::normalize(x.clone(), &self.norm2), &self.linear1, Then::Gelu);
        B::linear_layer(h, &self.linear2, Then::Add(x))
    }
}

/// `_ConvolutionalReductionModule`: Linear → GLU → depthwise strided Conv1d → LayerNorm → SiLU → Linear.
#[derive(Module, Debug)]
pub struct ConvReduction<B: Backend> {
    pub pre_linear: Linear<B>,
    pub conv_module: Conv1d<B>,
    pub post_norm: LayerNorm<B>,
    pub post_linear: Linear<B>,
    pub proj: usize,
}

impl<B: Backend> ConvReduction<B> {
    fn new(input: usize, proj: usize, kernel: usize, device: &B::Device) -> Self {
        ConvReduction {
            pre_linear: LinearConfig::new(input, 2 * proj).init(device),
            conv_module: Conv1dConfig::new(proj, proj, kernel)
                .with_stride(2)
                .with_groups(proj)
                .with_padding(PaddingConfig1d::Explicit((kernel - 1) / 2, (kernel - 1) / 2))
                .init(device),
            post_norm: layer_norm(proj, device),
            post_linear: LinearConfig::new(proj, proj).init(device),
            proj,
        }
    }
}

impl<B: Ops> ConvReduction<B> {
    fn forward(&self, x: Tensor<B, 3>) -> Tensor<B, 3> {
        let x = B::linear_layer(x, &self.pre_linear, Then::Keep);
        let glu = x.clone().narrow(2, 0, self.proj) * sigmoid(x.narrow(2, self.proj, self.proj));
        let conv = self.conv_module.forward(glu.swap_dims(1, 2)).swap_dims(1, 2);
        B::linear_layer(silu(B::normalize(conv, &self.post_norm)), &self.post_linear, Then::Keep)
    }
}

/// `_SelfAttentionPooling.W`.
#[derive(Module, Debug)]
pub struct Pooling<B: Backend> {
    pub w: Linear<B>,
}

/// `_TorchAttentionPoolingClassifier`.
#[derive(Module, Debug)]
pub struct Head<B: Backend> {
    pub conv_reduction1: ConvReduction<B>,
    pub mid_layer_norm1: LayerNorm<B>,
    pub mid_attention1: Mha<B>,
    pub pre_conv2_layer_norm: LayerNorm<B>,
    pub conv_reduction2: ConvReduction<B>,
    pub mid_layer_norm2: LayerNorm<B>,
    pub mid_attention2: Mha<B>,
    pub pre_conv3_layer_norm: LayerNorm<B>,
    pub conv_reduction3: ConvReduction<B>,
    pub pre_pooling_layer_norm: LayerNorm<B>,
    pub pooling_attention: Pooling<B>,
    pub classifier: Linear<B>,
    pub language_projector: Linear<B>,
    pub language_heads: Linear<B>,
}

/// The full model's parameters (`model.safetensors`).
#[derive(Module, Debug)]
pub struct Model<B: Backend> {
    pub feature_extractor: FeatureExtractor<B>,
    pub conv1: Conv1d<B>,
    pub conv2: Conv1d<B>,
    pub embed_positions: Positions<B>,
    pub attn_mask_conv: MaskConv<B>,
    pub layers: Vec<EncoderLayer<B>>,
    pub final_layer_norm: LayerNorm<B>,
    pub classifier: Head<B>,
}

/// The fixed sinusoidal position table (`embed_positions.weight`).
#[derive(Module, Debug)]
pub struct Positions<B: Backend> {
    pub weight: Param<Tensor<B, 2>>,
}

/// The fixed convolution that strides the attention mask (`attn_mask_conv`).
#[derive(Module, Debug)]
pub struct MaskConv<B: Backend> {
    pub weight: Param<Tensor<B, 3>>,
    pub bias: Param<Tensor<B, 1>>,
}

fn param<B: Backend, const D: usize>(shape: [usize; D], device: &B::Device) -> Param<Tensor<B, D>> {
    Param::from_tensor(Tensor::zeros(shape, device))
}

impl<B: Backend> Model<B> {
    /// The module tree with placeholder values, shaped by `cfg`; real values come from the checkpoint.
    pub fn new(cfg: &ModelConfig, device: &B::Device) -> Self {
        let (d, p, heads) = (cfg.hidden_size, cfg.classifier_proj_size, cfg.num_attention_heads);
        let bins = cfg.n_fft / 2 + 1;
        let head = Head {
            conv_reduction1: ConvReduction::new(d, d, 7, device),
            mid_layer_norm1: layer_norm(d, device),
            mid_attention1: Mha::new(d, 16, device),
            pre_conv2_layer_norm: layer_norm(d, device),
            conv_reduction2: ConvReduction::new(d, p, 7, device),
            mid_layer_norm2: layer_norm(p, device),
            mid_attention2: Mha::new(p, 16, device),
            pre_conv3_layer_norm: layer_norm(p, device),
            conv_reduction3: ConvReduction::new(p, p, 5, device),
            pre_pooling_layer_norm: layer_norm(p, device),
            pooling_attention: Pooling {
                w: LinearConfig::new(p, 1).init(device),
            },
            classifier: LinearConfig::new(p, cfg.num_labels).init(device),
            language_projector: LinearConfig::new(d, p).init(device),
            language_heads: LinearConfig::new(p, cfg.num_language_heads).init(device),
        };
        Model {
            feature_extractor: FeatureExtractor {
                fb: param([cfg.n_mels, bins], device),
                window: param([cfg.n_fft], device),
            },
            conv1: Conv1dConfig::new(cfg.n_mels, d, 3)
                .with_padding(PaddingConfig1d::Explicit(1, 1))
                .init(device),
            conv2: Conv1dConfig::new(d, d, 3)
                .with_stride(2)
                .with_padding(PaddingConfig1d::Explicit(1, 1))
                .init(device),
            embed_positions: Positions {
                weight: param([cfg.max_positions, d], device),
            },
            attn_mask_conv: MaskConv {
                weight: param([1, 1, 3], device),
                bias: param([1], device),
            },
            layers: (0..cfg.num_hidden_layers)
                .map(|_| EncoderLayer::new(d, heads, device))
                .collect(),
            final_layer_norm: layer_norm(d, device),
            classifier: head,
        }
    }
}

/// Tensors along the way, for golden tests and diagnostics.
#[derive(Debug)]
pub struct Trace<B: Backend> {
    pub logmel: Tensor<B, 3>,
    /// `conv2` output before GELU, `[1, d, T/2]`.
    pub conv2: Tensor<B, 3>,
    /// Output of each time-reduction pooling, `[1, d, L]` (PyTorch layout).
    pub pools: Vec<Tensor<B, 3>>,
    pub final_ln: Tensor<B, 3>,
    pub logits: Tensor<B, 1>,
    pub language_logits: Tensor<B, 1>,
}

/// The model ready to run (front end built, mask-conv weights read).
#[derive(Debug)]
pub struct Runner<B: Backend> {
    pub model: Model<B>,
    device: B::Device,
    frontend: Frontend<B>,
    mask_weight: [f32; 3],
    mask_bias: f32,
    hop: usize,
    reductions: Vec<(usize, usize)>,
}

impl<B: Ops> Runner<B> {
    pub fn new(model: Model<B>, cfg: &ModelConfig) -> Self {
        let frontend = Frontend::new(&model.feature_extractor, cfg.n_fft, cfg.hop_length);
        let device = model.feature_extractor.window.val().device();
        let w: Vec<f32> = model
            .attn_mask_conv
            .weight
            .val()
            .into_data()
            .to_vec()
            .unwrap_or_default();
        let b: Vec<f32> = model.attn_mask_conv.bias.val().into_data().to_vec().unwrap_or_default();
        Runner {
            model,
            device,
            frontend,
            mask_weight: [w[0], w[1], w[2]],
            mask_bias: b[0],
            hop: cfg.hop_length,
            reductions: cfg
                .time_reduction
                .iter()
                .map(|[layer, ratio]| (*layer, *ratio))
                .collect(),
        }
    }

    /// One clip (16 kHz samples in [-1, 1]) → logits and language logits (and the tensors along the way).
    pub fn forward(&self, pcm: &[f32]) -> Trace<B> {
        self.forward_inner(pcm, &mut None)
    }

    /// Like [`Runner::forward`], also returning how long each stage took (the device is synchronised between stages).
    pub fn forward_profiled(&self, pcm: &[f32]) -> (Trace<B>, Vec<(String, std::time::Duration)>) {
        let mut stages = Some((Vec::new(), std::time::Instant::now()));
        let trace = self.forward_inner(pcm, &mut stages);
        (trace, stages.map(|(s, _)| s).unwrap_or_default())
    }

    fn forward_inner(
        &self,
        pcm: &[f32],
        stages: &mut Option<(Vec<(String, std::time::Duration)>, std::time::Instant)>,
    ) -> Trace<B> {
        let device = self.device.clone();
        let mut mark = |name: &str| {
            if let Some((list, since)) = stages.as_mut() {
                let _ = B::sync(&device);
                list.push((name.to_owned(), since.elapsed()));
                *since = std::time::Instant::now();
            }
        };
        let m = &self.model;

        // Frame by frame, then token by token: [1, steps, channels].
        let mel = self.frontend.forward(pcm, &device);
        mark("front end");
        let valid0 = mask::after_frontend(pcm.len(), self.hop);
        debug_assert_eq!(valid0.len(), mel.dims()[1]);

        let h = B::conv_layer(mel.clone(), &m.conv1, Then::Gelu);
        let conv2 = B::conv_layer(h, &m.conv2, Then::Keep);
        let mut valid = mask::after_stride_conv(&valid0, self.mask_weight, self.mask_bias);
        let [_, l, d] = conv2.dims();
        let positions = m.embed_positions.weight.val().narrow(0, 0, l).reshape([1, l, d]);
        let mut h = gelu(conv2.clone()) + positions;
        let conv2 = conv2.swap_dims(1, 2);
        mark("conv1 + conv2");

        let mut pools = Vec::new();
        let mut keys = B::keys(&valid, &device);
        for (i, layer) in m.layers.iter().enumerate() {
            h = layer.forward(h, &keys);
            mark(&format!("layer {i}"));
            if let Some((_, ratio)) = self.reductions.iter().find(|(at, _)| *at == i) {
                // `avg_pool1d(kernel = stride = ratio)`: the mean of each group of `ratio` tokens, an incomplete
                // last group dropped.
                let [_, l, d] = h.dims();
                let len = l / ratio;
                h = h
                    .narrow(1, 0, len * ratio)
                    .reshape([1, len, *ratio, d])
                    .mean_dim(2)
                    .reshape([1, len, d]);
                pools.push(h.clone().swap_dims(1, 2));
                valid = mask::every(&valid, *ratio, len);
                keys = B::keys(&valid, &device);
            }
        }
        let h = B::normalize(h, &m.final_layer_norm);
        let final_ln = h.clone();
        let len = h.dims()[1];
        valid.truncate(len);
        let (logits, language_logits) = self.head(h, &valid, &device);
        mark("head");
        Trace {
            logmel: mel.swap_dims(1, 2),
            conv2,
            pools,
            final_ln,
            logits,
            language_logits,
        }
    }

    fn head(&self, h: Tensor<B, 3>, valid: &[bool], device: &B::Device) -> (Tensor<B, 1>, Tensor<B, 1>) {
        let c = &self.model.classifier;
        let original = h.clone();

        let h = c.conv_reduction1.forward(h);
        let valid1 = mask::every(valid, 2, usize::MAX);
        let h = c.mid_attention1.forward(
            B::normalize(h.clone(), &c.mid_layer_norm1),
            &B::keys(&valid1, device),
            h,
        );
        let h = c.conv_reduction2.forward(B::normalize(h, &c.pre_conv2_layer_norm));
        let valid2 = mask::every(&valid1, 2, usize::MAX);
        let h = c.mid_attention2.forward(
            B::normalize(h.clone(), &c.mid_layer_norm2),
            &B::keys(&valid2, device),
            h,
        );
        let h = c.conv_reduction3.forward(B::normalize(h, &c.pre_conv3_layer_norm));
        let valid3 = mask::every(&valid2, 2, usize::MAX);
        let h = B::normalize(h, &c.pre_pooling_layer_norm);

        let l = h.dims()[1];
        let weights = B::linear_layer(h.clone(), &c.pooling_attention.w, Then::Keep).reshape([1, l]);
        let weights = match key_padding::<B>(&valid3, device) {
            Some(pad) => weights.mask_fill(pad.reshape([1, l]), f32::NEG_INFINITY),
            None => weights,
        };
        let weights = softmax(weights, 1).reshape([1, l, 1]);
        let pooled = (h * weights).sum_dim(1);
        let logits = B::linear_layer(pooled, &c.classifier, Then::Keep).reshape([c.classifier.weight.dims()[1]]);

        let proj = B::linear_layer(original, &c.language_projector, Then::Keep);
        let l0 = proj.dims()[1];
        let ones: Vec<f32> = valid.iter().map(|v| if *v { 1.0 } else { 0.0 }).collect();
        let count: f32 = ones.iter().sum();
        let lang_mask = Tensor::<B, 1>::from_data(TensorData::new(ones, [l0]), device).reshape([1, l0, 1]);
        let pooled_lang = (proj * lang_mask).sum_dim(1) / count;
        let language_logits =
            B::linear_layer(pooled_lang, &c.language_heads, Then::Keep).reshape([c.language_heads.weight.dims()[1]]);
        (logits, language_logits)
    }
}
