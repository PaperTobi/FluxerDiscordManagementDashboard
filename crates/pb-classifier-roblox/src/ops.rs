//! The operations the GPU runs with kernels of its own (see `gpu.rs`). The CPU computes them from Burn's standard
//! operations (the `reference_*` functions).

use burn::nn::conv::Conv1d;
use burn::nn::{LayerNorm, LayerNormConfig, Linear};
use burn::tensor::activation::gelu;
use burn::tensor::backend::Backend;
use burn::tensor::module::attention;
use burn::tensor::ops::AttentionModuleOptions;
use burn::tensor::{Bool, Tensor, TensorData};

/// The epsilon of every layer normalisation in this model (PyTorch's default, which `inference.py` keeps).
pub const LAYER_NORM_EPSILON: f64 = 1e-5;

/// A layer normalisation over `d` values with [`LAYER_NORM_EPSILON`].
pub fn layer_norm<B: Backend>(d: usize, device: &B::Device) -> LayerNorm<B> {
    LayerNormConfig::new(d).with_epsilon(LAYER_NORM_EPSILON).init(device)
}

/// What happens to a linear layer's output before it is stored.
#[derive(Debug)]
pub enum Then<B: Backend> {
    /// Nothing.
    Keep,
    /// The exact (erf) GELU.
    Gelu,
    /// Adding a tensor of the output's shape (a residual connection).
    Add(Tensor<B, 3>),
}

/// A Burn backend with the operations of this model.
pub trait Ops: Backend {
    /// Which keys self-attention ignores, in the form the backend wants; built once per sequence by [`Ops::keys`].
    type Keys: Clone + std::fmt::Debug;

    /// The keys to ignore: `valid[i]` false means key `i` is padding.
    fn keys(valid: &[bool], device: &Self::Device) -> Self::Keys;

    /// Multi-head self-attention. `qkv`: `[batch, L, 3·d]`, queries, keys and values side by side (as the fused input
    /// projection of `nn.MultiheadAttention` makes them); the result is `[batch, L, d]`, heads side by side.
    fn self_attention(qkv: Tensor<Self, 3>, heads: usize, keys: &Self::Keys) -> Tensor<Self, 3>;

    /// `x · weight + bias` for `x` of shape `[batch, rows, in]`, then `then`.
    fn linear_layer(x: Tensor<Self, 3>, linear: &Linear<Self>, then: Then<Self>) -> Tensor<Self, 3> {
        reference_linear(x, linear, then)
    }

    /// `conv` over `x` given step by step (`[batch, steps, channels]`, the layout of the encoder's tokens), giving
    /// `[batch, steps', out]`, then `then`.
    fn conv_layer(x: Tensor<Self, 3>, conv: &Conv1d<Self>, then: Then<Self>) -> Tensor<Self, 3> {
        reference_conv(x, conv, then)
    }

    /// `norm(x)`, for a `norm` built by [`layer_norm`].
    fn normalize(x: Tensor<Self, 3>, norm: &LayerNorm<Self>) -> Tensor<Self, 3> {
        norm.forward(x)
    }
}

impl Ops for burn::backend::Flex {
    type Keys = Option<Tensor<Self, 4, Bool>>;

    fn keys(valid: &[bool], device: &Self::Device) -> Self::Keys {
        key_padding(valid, device)
    }

    fn self_attention(qkv: Tensor<Self, 3>, heads: usize, keys: &Self::Keys) -> Tensor<Self, 3> {
        reference_self_attention(qkv, heads, keys.as_ref())
    }
}

/// `[1, 1, 1, L]`, true for each key that is padding; `None` when every key is valid.
pub fn key_padding<B: Backend>(valid: &[bool], device: &B::Device) -> Option<Tensor<B, 4, Bool>> {
    if valid.iter().all(|v| *v) {
        return None;
    }
    let pad: Vec<bool> = valid.iter().map(|v| !v).collect();
    let n = pad.len();
    Some(Tensor::<B, 1, Bool>::from_data(TensorData::new(pad, [n]), device).reshape([1, 1, 1, n]))
}

/// [`Ops::self_attention`] in Burn's standard operations; `pad` from [`key_padding`].
pub fn reference_self_attention<B: Backend>(
    qkv: Tensor<B, 3>,
    heads: usize,
    pad: Option<&Tensor<B, 4, Bool>>,
) -> Tensor<B, 3> {
    let [b, l, width] = qkv.dims();
    let d = width / 3;
    let split = |i: usize| {
        qkv.clone()
            .narrow(2, i * d, d)
            .reshape([b, l, heads, d / heads])
            .swap_dims(1, 2)
    };
    let (q, k, v) = (split(0), split(1), split(2));
    let mask = pad.map(|p| p.clone().expand([b, heads, l, l]));
    let out = attention(q, k, v, mask, None, AttentionModuleOptions::default());
    out.swap_dims(1, 2).reshape([b, l, d])
}

/// [`Ops::linear_layer`] in Burn's standard operations.
pub fn reference_linear<B: Backend>(x: Tensor<B, 3>, linear: &Linear<B>, then: Then<B>) -> Tensor<B, 3> {
    then.apply(linear.forward(x))
}

/// [`Ops::conv_layer`] in Burn's standard operations.
pub fn reference_conv<B: Backend>(x: Tensor<B, 3>, conv: &Conv1d<B>, then: Then<B>) -> Tensor<B, 3> {
    then.apply(conv.forward(x.swap_dims(1, 2)).swap_dims(1, 2))
}

impl<B: Backend> Then<B> {
    fn apply(self, y: Tensor<B, 3>) -> Tensor<B, 3> {
        match self {
            Then::Keep => y,
            Then::Gelu => gelu(y),
            Then::Add(residual) => residual + y,
        }
    }
}
