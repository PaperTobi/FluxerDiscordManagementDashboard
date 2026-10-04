//! GPU kernels (CubeCL) for the operations in [`Ops`]. Burn runs every tensor operation as a kernel of its own;
//! these few cover most of the model's work in far fewer launches, and their compiled form does not depend on the
//! clip length, so a new length compiles nothing.
//!
//! - Linear layers (and the convolutions in front of the encoder, as linear layers over their windows): a speech
//!   segment is short, so a layer multiplies a few dozen rows by a large weight matrix. Burn's matrix product splits
//!   the work over output tiles only; with so few rows that left most of the GPU idle while each unit walked the whole
//!   inner dimension, and every layer cost the same few milliseconds whatever the clip length. Here each unit computes
//!   `TILE` rows by four columns over a slice of the inner dimension, and the slices are added in a second pass when
//!   there are too few rows and columns to fill the GPU. Bias, GELU and residual connections are applied as the result
//!   is stored.
//! - Layer normalisation: one cube per row.
//! - Self-attention: one kernel reading queries, keys and values straight from the input projection's output.
//!
//! The kernels are launched with bounds checks (`launch`, not `launch_unchecked`): this crate has no `unsafe`.

use burn::nn::conv::Conv1d;
use burn::nn::{LayerNorm, Linear, PaddingConfig1d};
use burn::tensor::ops::PadMode;
use burn::tensor::{DType, Shape, Tensor, TensorData, TensorPrimitive};
use burn_cubecl::kernel::into_contiguous;
use burn_cubecl::ops::numeric::empty_device_contiguous_dtype;
use burn_cubecl::tensor::CubeTensor;
use burn_cubecl::{BoolElement, CubeBackend, CubeRuntime, IntElement};
use cubecl::prelude::*;

use crate::ops::{LAYER_NORM_EPSILON, Ops, Then, reference_conv, reference_linear, reference_self_attention};

/// Rows each unit computes.
const TILE: usize = 8;
/// Units per cube (along the output columns).
const UNITS: u32 = 64;
/// Units worth keeping busy: below this the inner dimension is split.
const BUSY_UNITS: usize = 65_536;
/// Shortest slice of the inner dimension worth a pass of its own.
const MIN_SLICE: usize = 64;
/// Output columns per unit (one vector), and values per vector elsewhere.
const VECTOR: usize = 4;
/// Units per cube of [`linear_sum_kernel`].
const SUM_UNITS: usize = 256;

/// [`Then`] as the kernels see it.
const KEEP: u32 = 0;
const GELU: u32 = 1;
const ADD: u32 = 2;

/// The value stored for one output vector: `sum + bias`, then the epilogue.
#[cube]
fn finish<N: Size>(
    sum: Vector<f32, N>,
    bias: Vector<f32, N>,
    residual: &Array<Vector<f32, N>>,
    at: usize,
    #[comptime] then: u32,
) -> Vector<f32, N> {
    let y = sum + bias;
    if comptime!(then == GELU) {
        let half = Vector::new(0.5f32);
        y * (Vector::erf(y * Vector::new(core::f32::consts::FRAC_1_SQRT_2)) + Vector::new(1.0f32)) * half
    } else if comptime!(then == ADD) {
        residual[at] + y
    } else {
        y
    }
}

/// `x [rows, k] · w [k, cols·N]` for rows `CUBE_POS_Y·tile..` and the slice `CUBE_POS_Z·slice..` of `k` (`N` values
/// of `k` at a time). Writes the finished output (`split` false) or the slice's partial sum to `out[CUBE_POS_Z]`
/// (`split` true).
#[cube(launch)]
#[allow(clippy::too_many_arguments)]
fn linear_kernel<N: Size>(
    x: &Array<Vector<f32, N>>,
    w: &Array<Vector<f32, N>>,
    bias: &Array<Vector<f32, N>>,
    residual: &Array<Vector<f32, N>>,
    out: &mut Array<Vector<f32, N>>,
    rows: u32,
    k: u32,
    cols: u32,
    slice: u32,
    #[comptime] tile: usize,
    #[comptime] split: bool,
    #[comptime] then: u32,
) {
    let col = (CUBE_POS_X * CUBE_DIM_X + UNIT_POS_X) as usize;
    if col >= cols as usize {
        terminate!();
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let width = N::value();
    let k_vectors = k as usize / width;
    let row0 = CUBE_POS_Y as usize * tile;
    let first = CUBE_POS_Z as usize * slice as usize / width;
    let last = min(first + slice as usize / width, k_vectors);

    let mut acc = Array::<Vector<f32, N>>::new(tile);
    #[unroll]
    for r in 0..tile {
        acc[r] = Vector::new(0.0f32);
    }
    for v in first..last {
        // `width` rows of `w` (one vector each), then `width` values of `k` from each row of `x`.
        let mut wv = Array::<Vector<f32, N>>::new(width);
        #[unroll]
        for j in 0..width {
            wv[j] = w[(v * width + j) * cols + col];
        }
        #[unroll]
        for r in 0..tile {
            // Rows past the end repeat the last row; they are computed but never stored.
            let xv = x[min(row0 + r, rows - 1) * k_vectors + v];
            #[unroll]
            for j in 0..width {
                acc[r] += Vector::new(xv[j]) * wv[j];
            }
        }
    }
    #[unroll]
    for r in 0..tile {
        let row = row0 + r;
        if row < rows {
            if comptime!(split) {
                out[(CUBE_POS_Z as usize * rows + row) * cols + col] = acc[r];
            } else {
                out[row * cols + col] = finish::<N>(acc[r], bias[col], residual, row * cols + col, then);
            }
        }
    }
}

/// Adds the `slices` partial sums of [`linear_kernel`] and finishes the output.
#[cube(launch)]
fn linear_sum_kernel<N: Size>(
    partial: &Array<Vector<f32, N>>,
    bias: &Array<Vector<f32, N>>,
    residual: &Array<Vector<f32, N>>,
    out: &mut Array<Vector<f32, N>>,
    cols: u32,
    slices: u32,
    #[comptime] then: u32,
) {
    let at = ABSOLUTE_POS;
    let len = out.len();
    if at >= len {
        terminate!();
    }
    let mut sum = partial[at];
    for s in 1..slices as usize {
        sum += partial[s * len + at];
    }
    out[at] = finish::<N>(sum, bias[at % cols as usize], residual, at, then);
}

/// Units per cube of [`layer_norm_kernel`] (a power of two).
const NORM_UNITS: usize = 256;

/// Sum of `value` over the cube's units (a power of two, one `shared` slot each); every unit gets the total.
#[cube]
fn cube_sum(shared: &mut SharedMemory<f32>, value: f32) -> f32 {
    let unit = UNIT_POS_X as usize;
    shared[unit] = value;
    sync_cube();
    let mut active = CUBE_DIM_X as usize / 2;
    while active > 0 {
        if unit < active {
            let other = shared[unit + active];
            shared[unit] += other;
        }
        sync_cube();
        active /= 2;
    }
    let total = shared[0];
    sync_cube();
    total
}

/// Layer normalisation of row `CUBE_POS_X` (`cols` vectors of `width` values in all): the mean, the mean squared
/// deviation from it, then `(x - mean) / sqrt(var + epsilon) · gamma + beta`, as Burn computes it.
#[cube(launch)]
fn layer_norm_kernel<N: Size>(
    x: &Array<Vector<f32, N>>,
    gamma: &Array<Vector<f32, N>>,
    beta: &Array<Vector<f32, N>>,
    out: &mut Array<Vector<f32, N>>,
    cols: u32,
    width: f32,
    epsilon: f32,
    #[comptime] units: usize,
) {
    let cols = cols as usize;
    let base = CUBE_POS_X as usize * cols;
    let mut shared = SharedMemory::<f32>::new(units);

    let mut sum = 0.0f32;
    let mut i = UNIT_POS_X as usize;
    while i < cols {
        let v = x[base + i];
        #[unroll]
        for j in 0..N::value() {
            sum += v[j];
        }
        i += units;
    }
    let mean = Vector::new(cube_sum(&mut shared, sum) / width);

    let mut squares = 0.0f32;
    let mut i = UNIT_POS_X as usize;
    while i < cols {
        let c = x[base + i] - mean;
        let c = c * c;
        #[unroll]
        for j in 0..N::value() {
            squares += c[j];
        }
        i += units;
    }
    let var = cube_sum(&mut shared, squares) / width;
    let denom = Vector::sqrt(Vector::new(var + epsilon));

    let mut i = UNIT_POS_X as usize;
    while i < cols {
        out[base + i] = (x[base + i] - mean) / denom * gamma[i] + beta[i];
        i += units;
    }
}

/// Queries per cube of [`attention_kernel`], and keys per tile it loads.
const ATTENTION_BLOCK: usize = 64;
/// Keys [`attention_kernel`] scores before it weighs their values.
const ATTENTION_GROUP: usize = 8;

/// Self-attention for one head (`CUBE_POS_Y`) and the queries `CUBE_POS_X·block..`, one query per unit. `qkv` holds
/// `len` rows of queries, keys and values side by side (`3·heads·head` vectors); keys with `ignored[i] = 1` take no
/// part; `scale` multiplies the queries. Keys and values are loaded a tile of `block` at a time into shared memory, and the softmax is accumulated
/// online, `group` keys at a time: their scores, the running maximum, then their weighted values.
#[cube(launch)]
#[allow(clippy::too_many_arguments)]
fn attention_kernel<N: Size>(
    qkv: &Array<Vector<f32, N>>,
    ignored: &Array<f32>,
    out: &mut Array<Vector<f32, N>>,
    len: u32,
    scale: f32,
    #[comptime] head: usize,
    #[comptime] block: usize,
    #[comptime] group: usize,
) {
    let len = len as usize;
    let unit = UNIT_POS_X as usize;
    let d = CUBE_COUNT_Y as usize * head;
    let row = 3 * d;
    let offset = CUBE_POS_Y as usize * head;
    let query = CUBE_POS_X as usize * block + unit;
    // Units past the last query follow the last one (their result is not stored), so every unit takes part in
    // loading the tiles.
    let from = min(query, len - 1) * row + offset;

    let mut q = Array::<Vector<f32, N>>::new(head);
    let mut acc = Array::<Vector<f32, N>>::new(head);
    #[unroll]
    for i in 0..head {
        q[i] = qkv[from + i] * Vector::new(scale);
        acc[i] = Vector::new(0.0f32);
    }
    let mut keys = SharedMemory::<Vector<f32, N>>::new(block * head);
    let mut values = SharedMemory::<Vector<f32, N>>::new(block * head);
    let mut skip = SharedMemory::<f32>::new(block);
    let mut scores = Array::<f32>::new(group);
    let lowest = f32::new(-3.0e38f32);
    let mut top = lowest;
    let mut total = 0.0f32;

    let mut first = 0;
    while first < len {
        // Keys past the end are zeros and ignored.
        let key = first + unit;
        let inside = key < len;
        let at = min(key, len - 1) * row + offset;
        #[unroll]
        for i in 0..head {
            keys[unit * head + i] = select(inside, qkv[at + d + i], Vector::new(0.0f32));
            values[unit * head + i] = select(inside, qkv[at + 2 * d + i], Vector::new(0.0f32));
        }
        skip[unit] = select(inside, ignored[min(key, len - 1)], 1.0f32);
        sync_cube();

        for g in 0..block / group {
            let mut group_top = top;
            #[unroll]
            for e in 0..group {
                let j = g * group + e;
                let mut dot = Vector::new(0.0f32);
                #[unroll]
                for i in 0..head {
                    dot += q[i] * keys[j * head + i];
                }
                let mut score = 0.0f32;
                #[unroll]
                for c in 0..N::value() {
                    score += dot[c];
                }
                scores[e] = select(skip[j] == 0.0f32, score, lowest);
                group_top = max(group_top, scores[e]);
            }
            let fade = (top - group_top).exp();
            total *= fade;
            #[unroll]
            for i in 0..head {
                acc[i] *= Vector::new(fade);
            }
            #[unroll]
            for e in 0..group {
                let j = g * group + e;
                let weight = select(skip[j] == 0.0f32, (scores[e] - group_top).exp(), 0.0f32);
                total += weight;
                #[unroll]
                for i in 0..head {
                    acc[i] += values[j * head + i] * Vector::new(weight);
                }
            }
            top = group_top;
        }
        sync_cube();
        first += block;
    }
    if query < len {
        let norm = Vector::new(1.0f32 / total);
        #[unroll]
        for i in 0..head {
            out[query * d + offset + i] = acc[i] * norm;
        }
    }
}

/// The tensor's storage, laid out row by row (copied if it was not).
fn prim<R: CubeRuntime, BT: BoolElement, I: IntElement, const D: usize>(
    t: Tensor<CubeBackend<R, f32, I, BT>, D>,
) -> CubeTensor<R> {
    into_contiguous(t.into_primitive().tensor())
}

/// `x [batch, rows, k] · w [k, n] + bias`, then `then`, with [`linear_kernel`]: [`VECTOR`] values at a time when `k`
/// and `n` allow it, else one.
fn matmul_bias<R: CubeRuntime, I: IntElement, BT: BoolElement>(
    x: Tensor<CubeBackend<R, f32, I, BT>, 3>,
    w: Tensor<CubeBackend<R, f32, I, BT>, 2>,
    bias: Tensor<CubeBackend<R, f32, I, BT>, 1>,
    then: Then<CubeBackend<R, f32, I, BT>>,
) -> Tensor<CubeBackend<R, f32, I, BT>, 3> {
    let [batch, rows, k] = x.dims();
    let n = w.dims()[1];
    let rows = batch * rows;
    let vector = if k.is_multiple_of(VECTOR) && n.is_multiple_of(VECTOR) {
        VECTOR
    } else {
        1
    };
    let cols = n / vector;
    let x = prim(x);
    let client = x.client.clone();
    let device = x.device.clone();
    let w = prim(w);
    let bias = prim(bias);
    let (code, residual) = match then {
        Then::Keep => (KEEP, None),
        Then::Gelu => (GELU, None),
        Then::Add(r) => (ADD, Some(prim(r))),
    };
    // When there is nothing to add, the bias stands in for the residual argument (never read).
    let residual = residual.unwrap_or_else(|| bias.clone());
    let out = empty_device_contiguous_dtype(
        client.clone(),
        device.clone(),
        Shape::new([batch, rows / batch, n]),
        DType::F32,
    );

    let row_tiles = rows.div_ceil(TILE);
    let busy = cols * row_tiles;
    let slices = (BUSY_UNITS / busy).clamp(1, (k / MIN_SLICE).max(1));
    // Slices are whole vectors of `k`.
    let slice = k.div_ceil(slices).next_multiple_of(vector);
    let slices = k.div_ceil(slice);
    let cube_count = CubeCount::Static(cols.div_ceil(UNITS as usize) as u32, row_tiles as u32, slices as u32);
    let split = slices > 1;
    let partial =
        split.then(|| empty_device_contiguous_dtype(client.clone(), device, Shape::new([slices, rows, n]), DType::F32));
    linear_kernel::launch::<R>(
        &client,
        cube_count,
        CubeDim::new_1d(UNITS),
        vector,
        x.into_array_arg(),
        w.into_array_arg(),
        bias.clone().into_array_arg(),
        residual.clone().into_array_arg(),
        partial.as_ref().unwrap_or(&out).clone().into_array_arg(),
        rows as u32,
        k as u32,
        cols as u32,
        slice as u32,
        TILE,
        split,
        code,
    );
    if let Some(partial) = partial {
        linear_sum_kernel::launch::<R>(
            &client,
            CubeCount::Static((rows * cols).div_ceil(SUM_UNITS) as u32, 1, 1),
            CubeDim::new_1d(SUM_UNITS as u32),
            vector,
            partial.into_array_arg(),
            bias.into_array_arg(),
            residual.into_array_arg(),
            out.clone().into_array_arg(),
            cols as u32,
            slices as u32,
            code,
        );
    }
    Tensor::from_primitive(TensorPrimitive::Float(out))
}

impl<R: CubeRuntime, I: IntElement, BT: BoolElement> Ops for CubeBackend<R, f32, I, BT> {
    fn linear_layer(x: Tensor<Self, 3>, linear: &Linear<Self>, then: Then<Self>) -> Tensor<Self, 3> {
        match &linear.bias {
            Some(bias) if x.dims()[1] > 0 => matmul_bias(x, linear.weight.val(), bias.val(), then),
            _ => reference_linear(x, linear, then),
        }
    }

    /// As a linear layer over the windows the convolution sees: each output step is the weights times the `kernel`
    /// input steps under it, all channels.
    fn conv_layer(x: Tensor<Self, 3>, conv: &Conv1d<Self>, then: Then<Self>) -> Tensor<Self, 3> {
        let [out_channels, in_channels, kernel] = conv.weight.dims();
        let pad = match conv.padding {
            PaddingConfig1d::Explicit(left, right) if left == right => left,
            PaddingConfig1d::Valid => 0,
            _ => return reference_conv(x, conv, then),
        };
        let Some(bias) = conv.bias.as_ref().filter(|_| conv.groups == 1 && conv.dilation == 1) else {
            return reference_conv(x, conv, then);
        };
        if x.dims()[1] + 2 * pad < kernel {
            return reference_conv(x, conv, then);
        }
        let x = if pad > 0 {
            x.pad([(0, 0), (pad, pad), (0, 0)], PadMode::Constant(0.0))
        } else {
            x
        };
        // [batch, steps', in_channels, kernel]: the channels and taps in the order of the weights ([out, in, kernel]).
        let windows: Tensor<Self, 4> = x.unfold(1, kernel, conv.stride);
        let [batch, steps, _, _] = windows.dims();
        let x = windows.reshape([batch, steps, in_channels * kernel]);
        let w = conv
            .weight
            .val()
            .reshape([out_channels, in_channels * kernel])
            .transpose();
        matmul_bias(x, w, bias.val(), then)
    }

    /// `[L]`: 1 for each key that is padding, 0 for the others.
    type Keys = Tensor<Self, 1>;

    fn keys(valid: &[bool], device: &Self::Device) -> Self::Keys {
        let ignored: Vec<f32> = valid.iter().map(|v| if *v { 0.0 } else { 1.0 }).collect();
        let n = ignored.len();
        Tensor::from_data(TensorData::new(ignored, [n]), device)
    }

    fn self_attention(qkv: Tensor<Self, 3>, heads: usize, keys: &Self::Keys) -> Tensor<Self, 3> {
        let [batch, len, width] = qkv.dims();
        let d = width / 3;
        let head = d / heads;
        if batch != 1 || !head.is_multiple_of(VECTOR) || len == 0 {
            let pad = keys.clone().equal_elem(1.0).reshape([1, 1, 1, len]);
            return reference_self_attention(qkv, heads, Some(&pad));
        }
        let qkv = prim(qkv);
        let client = qkv.client.clone();
        let device = qkv.device.clone();
        let ignored = prim(keys.clone());
        let out = empty_device_contiguous_dtype(client.clone(), device, Shape::new([1, len, d]), DType::F32);
        attention_kernel::launch::<R>(
            &client,
            CubeCount::Static(len.div_ceil(ATTENTION_BLOCK) as u32, heads as u32, 1),
            CubeDim::new_1d(ATTENTION_BLOCK as u32),
            VECTOR,
            qkv.into_array_arg(),
            ignored.into_array_arg(),
            out.clone().into_array_arg(),
            len as u32,
            1.0 / (head as f32).sqrt(),
            head / VECTOR,
            ATTENTION_BLOCK,
            ATTENTION_GROUP,
        );
        Tensor::from_primitive(TensorPrimitive::Float(out))
    }

    fn normalize(x: Tensor<Self, 3>, norm: &LayerNorm<Self>) -> Tensor<Self, 3> {
        let [batch, rows, width] = x.dims();
        let Some(beta) = norm.beta.as_ref().map(|b| b.val()) else {
            return norm.forward(x);
        };
        if !width.is_multiple_of(VECTOR) {
            return norm.forward(x);
        }
        let x = prim(x);
        let client = x.client.clone();
        let out = empty_device_contiguous_dtype(
            client.clone(),
            x.device.clone(),
            Shape::new([batch, rows, width]),
            DType::F32,
        );
        layer_norm_kernel::launch::<R>(
            &client,
            CubeCount::Static((batch * rows) as u32, 1, 1),
            CubeDim::new_1d(NORM_UNITS as u32),
            VECTOR,
            x.into_array_arg(),
            prim(norm.gamma.val()).into_array_arg(),
            prim(beta).into_array_arg(),
            out.clone().into_array_arg(),
            (width / VECTOR) as u32,
            width as f32,
            LAYER_NORM_EPSILON as f32,
            NORM_UNITS,
        );
        Tensor::from_primitive(TensorPrimitive::Float(out))
    }
}
