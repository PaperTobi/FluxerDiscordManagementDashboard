//! Log-mel front end, the same computation as `_LogMelFeatureExtraction` in `inference.py`:
//! `torch.stft(center=True, reflect pad, hann window)`, drop the last frame, power spectrum, mel filterbank,
//! `log10(clamp(1e-10))`, clamp to `max - 8`, `(x + 4) / 4`.
//!
//! As in `torch.stft`, each frame is multiplied by the window first; the DFT is then a matrix product with a cos/sin
//! basis (built in f64): mathematically the same transform as torch's FFT, and it runs on whatever device the model
//! runs on. (Burn's own STFT only accepts power-of-two sizes; this model uses 400.)

use burn::module::{Module, Param};
use burn::tensor::backend::Backend;
use burn::tensor::ops::PadMode;
use burn::tensor::{ElementConversion, Tensor, TensorData};

/// The parameters stored in the checkpoint (`feature_extractor.fb`, `feature_extractor.window`).
#[derive(Module, Debug)]
pub struct FeatureExtractor<B: Backend> {
    /// Mel filterbank `[n_mels, n_fft / 2 + 1]`.
    pub fb: Param<Tensor<B, 2>>,
    /// Analysis window `[n_fft]`.
    pub window: Param<Tensor<B, 1>>,
}

/// The front end ready to run: the filterbank plus the DFT basis built from the loaded window.
#[derive(Debug, Clone)]
pub struct Frontend<B: Backend> {
    fb: Tensor<B, 3>,
    window: Tensor<B, 3>,
    basis: Tensor<B, 3>,
    n_fft: usize,
    hop: usize,
    bins: usize,
}

impl<B: Backend> Frontend<B> {
    pub fn new(fe: &FeatureExtractor<B>, n_fft: usize, hop: usize) -> Self {
        let fb = fe.fb.val();
        let device = fb.device();
        let [n_mels, bins] = fb.dims();
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
        let basis = Tensor::<B, 2>::from_data(TensorData::new(basis, [n_fft, 2 * bins]), &device).unsqueeze_dim(0);
        Frontend {
            fb: fb.reshape([1, n_mels, bins]),
            window: window.reshape([1, 1, n_fft]),
            basis,
            n_fft,
            hop,
            bins,
        }
    }

    /// `wav`: `[1, samples]` in [-1, 1]  →  log-mel `[1, n_mels, frames]`.
    pub fn forward(&self, wav: Tensor<B, 2>) -> Tensor<B, 3> {
        let half = self.n_fft / 2;
        let padded = wav.pad([(0, 0), (half, half)], PadMode::Reflect);
        let frames: Tensor<B, 3> = padded.unfold(1, self.n_fft, self.hop);
        let [_, count, _] = frames.dims();
        let windowed = frames.narrow(1, 0, count - 1) * self.window.clone();
        let spec = windowed.matmul(self.basis.clone());
        let re = spec.clone().narrow(2, 0, self.bins);
        let im = spec.narrow(2, self.bins, self.bins);
        let power = re.clone() * re + im.clone() * im;
        let mel = self.fb.clone().matmul(power.swap_dims(1, 2));
        let log10 = mel.clamp_min(1e-10).log() / std::f32::consts::LN_10;
        let top: f32 = log10.clone().max().into_scalar().elem();
        (log10.clamp_min(top - 8.0) + 4.0) / 4.0
    }
}
