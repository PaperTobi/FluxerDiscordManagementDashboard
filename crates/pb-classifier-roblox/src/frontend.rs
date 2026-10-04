//! Log-mel front end, the same computation as `_LogMelFeatureExtraction` in `inference.py`:
//! `torch.stft(center=True, reflect pad, hann window)`, drop the last frame, power spectrum, mel filterbank,
//! `log10(clamp(1e-10))`, clamp to `max - 8`, `(x + 4) / 4`.
//!
//! As in `torch.stft`, each frame is multiplied by the window first; the DFT is then a matrix product with a cos/sin
//! basis (built in f64): mathematically the same transform as torch's FFT, and it runs on whatever device the model
//! runs on. (Burn's own STFT only accepts power-of-two sizes; this model uses 400.) The DFT and the filterbank are
//! linear layers without bias, so they run like the model's other linear layers.

use burn::module::{Module, Param};
use burn::nn::Linear;
use burn::tensor::backend::Backend;
use burn::tensor::{Tensor, TensorData};

use crate::ops::{Ops, Then};

/// The parameters stored in the checkpoint (`feature_extractor.fb`, `feature_extractor.window`).
#[derive(Module, Debug)]
pub struct FeatureExtractor<B: Backend> {
    /// Mel filterbank `[n_mels, n_fft / 2 + 1]`.
    pub fb: Param<Tensor<B, 2>>,
    /// Analysis window `[n_fft]`.
    pub window: Param<Tensor<B, 1>>,
}

/// The front end ready to run: the window, the DFT basis and the filterbank.
#[derive(Debug)]
pub struct Frontend<B: Backend> {
    window: Tensor<B, 3>,
    /// `[n_fft, 2·bins]`: the real parts, then the imaginary parts.
    dft: Linear<B>,
    /// `[bins, n_mels]`.
    mel: Linear<B>,
    n_fft: usize,
    hop: usize,
    bins: usize,
}

/// A linear layer with weight `[inputs, outputs]` (given row by row) and no bias (a zero bias).
fn fixed_linear<B: Backend>(weight: Vec<f32>, inputs: usize, outputs: usize, device: &B::Device) -> Linear<B> {
    Linear {
        weight: Param::from_tensor(Tensor::from_data(TensorData::new(weight, [inputs, outputs]), device)),
        bias: Some(Param::from_tensor(Tensor::zeros([outputs], device))),
    }
}

/// `x` with `pad` samples mirrored onto each end, the edge itself not repeated (`torch.stft`'s reflect padding).
fn reflect(x: &[f32], pad: usize) -> Vec<f32> {
    let n = x.len();
    let mut out = Vec::with_capacity(n + 2 * pad);
    out.extend((1..=pad).rev().map(|i| x[i]));
    out.extend_from_slice(x);
    out.extend((1..=pad).map(|i| x[n - 1 - i]));
    out
}

impl<B: Ops> Frontend<B> {
    pub fn new(fe: &FeatureExtractor<B>, n_fft: usize, hop: usize) -> Self {
        let device = fe.fb.val().device();
        let [n_mels, bins] = fe.fb.dims();
        let window = fe.window.val();
        assert_eq!(window.dims()[0], n_fft, "window length");
        assert_eq!(bins, n_fft / 2 + 1, "filterbank width");
        // basis[n][k] = cos(2πkn/N), basis[n][bins + k] = −sin(2πkn/N)   (X[k] = Σ x[n]·e^{−2πikn/N})
        let mut basis = vec![0.0f32; n_fft * 2 * bins];
        for n in 0..n_fft {
            for k in 0..bins {
                let angle = 2.0 * std::f64::consts::PI * ((k * n) % n_fft) as f64 / n_fft as f64;
                basis[n * 2 * bins + k] = angle.cos() as f32;
                basis[n * 2 * bins + bins + k] = (-angle.sin()) as f32;
            }
        }
        let fb: Vec<f32> = fe.fb.val().into_data().to_vec().unwrap_or_default();
        let fb_t: Vec<f32> = (0..bins * n_mels)
            .map(|i| fb[(i % n_mels) * bins + i / n_mels])
            .collect();
        Frontend {
            window: window.reshape([1, 1, n_fft]),
            dft: fixed_linear(basis, n_fft, 2 * bins, &device),
            mel: fixed_linear(fb_t, bins, n_mels, &device),
            n_fft,
            hop,
            bins,
        }
    }

    /// `pcm`: samples in [-1, 1], more than `n_fft / 2`  →  log-mel `[1, frames, n_mels]`.
    pub fn forward(&self, pcm: &[f32], device: &B::Device) -> Tensor<B, 3> {
        let padded = reflect(pcm, self.n_fft / 2);
        let n = padded.len();
        let wav = Tensor::<B, 2>::from_data(TensorData::new(padded, [1, n]), device);
        let frames: Tensor<B, 3> = wav.unfold(1, self.n_fft, self.hop);
        let [_, count, _] = frames.dims();
        let windowed = frames.narrow(1, 0, count - 1) * self.window.clone();
        let spec = B::linear_layer(windowed, &self.dft, Then::Keep);
        let re = spec.clone().narrow(2, 0, self.bins);
        let im = spec.narrow(2, self.bins, self.bins);
        let power = re.clone() * re + im.clone() * im;
        let mel = B::linear_layer(power, &self.mel, Then::Keep);
        let log10 = mel.clamp_min(1e-10).log() / std::f32::consts::LN_10;
        // `clamp(min = max - 8)`, with the maximum kept on the device (reading it back would stall the GPU).
        let floor = (log10.clone().max() - 8.0).reshape([1, 1, 1]).expand(log10.dims());
        (log10.max_pair(floor) + 4.0) / 4.0
    }
}

#[cfg(test)]
mod tests {
    use super::reflect;

    #[test]
    fn reflect_mirrors_without_repeating_the_edge() {
        assert_eq!(
            reflect(&[0.0, 1.0, 2.0, 3.0], 2),
            [2.0, 1.0, 0.0, 1.0, 2.0, 3.0, 2.0, 1.0]
        );
    }
}
