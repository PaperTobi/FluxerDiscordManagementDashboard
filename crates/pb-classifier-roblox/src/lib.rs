//! The Roblox voice-safety classifier v3 (`Roblox/voice-safety-classifier-v3`, Apache-2.0) written in Burn and loaded
//! from the official `model.safetensors`. Computes the same numbers as Roblox's `inference.py` for one clip
//! (checked by the golden tests against the PyTorch reference). Runs on the CPU with Burn's pure-Rust backend, or
//! with the `gpu` feature on a GPU through wgpu (Vulkan).

mod config;
mod frontend;
mod mask;
pub mod model;

use std::num::NonZeroUsize;
use std::path::Path;

use burn::module::Module;
use burn::tensor::activation::{sigmoid, softmax};
use burn::tensor::backend::Backend;
use burn::tensor::{Tensor, TensorData};
use burn_store::{ModuleSnapshot, PyTorchToBurnAdapter, SafetensorsStore};
use pb_models_api::{Classifier, ClassifierInfo, ModelError, RawScores};

pub use config::{ConfigError, ModelConfig};
pub use model::{Model, Runner, Trace};

/// Burn's pure-Rust CPU backend.
pub type Cpu = burn::backend::Flex;
#[cfg(feature = "gpu")]
pub type Gpu = burn::backend::Wgpu;

/// Files in the model directory.
pub const CONFIG_FILE: &str = "config.json";
pub const WEIGHTS_FILE: &str = "model.safetensors";
pub const REVISION_FILE: &str = "REVISION";

/// The classifier, bound to one device. CPU work runs on its own thread pool, sized by `set_threads`.
pub struct RobloxClassifier<B: Backend> {
    runner: Runner<B>,
    cfg: ModelConfig,
    info: ClassifierInfo,
    device: B::Device,
    pool: Option<rayon::ThreadPool>,
}

impl<B: Backend> std::fmt::Debug for RobloxClassifier<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RobloxClassifier")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

fn pool(threads: NonZeroUsize) -> Result<rayon::ThreadPool, ModelError> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads.get())
        .thread_name(|i| format!("classify-{i}"))
        .build()
        .map_err(|e| ModelError::Load(format!("thread pool: {e}")))
}

/// Smallest input that leaves at least one unmasked token at every stage (shorter input would only give NaN).
fn min_samples(cfg: &ModelConfig) -> usize {
    (1..)
        .find(|&frames: &usize| {
            let mut len = frames.div_ceil(2);
            for [_, ratio] in &cfg.time_reduction {
                len /= ratio;
            }
            len >= 1
        })
        .map_or(usize::MAX, |frames| (frames * cfg.hop_length).max(cfg.n_fft / 2 + 1))
}

/// Loads the model directory (`config.json`, `model.safetensors`, optional `REVISION`) onto `device`.
pub fn load_runner<B: Backend>(dir: &Path, device: &B::Device) -> Result<(Runner<B>, ModelConfig), ModelError> {
    let cfg = ModelConfig::load(&dir.join(CONFIG_FILE)).map_err(|e| ModelError::Load(e.to_string()))?;
    let mut model = Model::<B>::new(&cfg, device);
    let mut store = SafetensorsStore::from_file(dir.join(WEIGHTS_FILE))
        .with_from_adapter(PyTorchToBurnAdapter)
        .with_key_remapping(r"in_proj_weight$", "in_proj.weight")
        .with_key_remapping(r"in_proj_bias$", "in_proj.bias")
        .with_key_remapping(r"pre_conv_seq\.0\.", "pre_linear.")
        .with_key_remapping(r"post_conv_seq\.0\.", "post_norm.")
        .with_key_remapping(r"post_conv_seq\.2\.", "post_linear.")
        .with_key_remapping(r"pooling_attention\.W\.", "pooling_attention.w.")
        // Every LayerNorm in this model is named `…norm`, `…norm1` or `…norm2`. Burn calls their parameters gamma/beta.
        // (Mapped here rather than through the adapter's alternative-name lookup, which burn-store 0.21 reports as
        // "unused" even when it was applied, and that would defeat the strict check below.)
        .with_key_remapping(r"(norm\d?)\.weight$", "${1}.gamma")
        .with_key_remapping(r"(norm\d?)\.bias$", "${1}.beta");
    let result = model
        .load_from(&mut store)
        .map_err(|e| ModelError::Load(format!("{e:?}")))?;
    if !result.missing.is_empty() || !result.unused.is_empty() || !result.errors.is_empty() {
        return Err(ModelError::Load(format!(
            "checkpoint does not match the model: missing {:?}, unused {:?}, errors {:?}",
            result.missing, result.unused, result.errors
        )));
    }
    let model = model.no_grad();
    Ok((Runner::new(model, &cfg), cfg))
}

impl<B: Backend> RobloxClassifier<B> {
    fn build(
        dir: &Path,
        device: B::Device,
        device_name: String,
        threads: Option<NonZeroUsize>,
    ) -> Result<Self, ModelError> {
        let (runner, cfg) = load_runner::<B>(dir, &device)?;
        let revision = std::fs::read_to_string(dir.join(REVISION_FILE))
            .map(|r| r.trim().to_owned())
            .unwrap_or_default();
        let model = if revision.is_empty() {
            "Roblox/voice-safety-classifier-v3".to_owned()
        } else {
            format!("Roblox/voice-safety-classifier-v3@{revision}")
        };
        let info = ClassifierInfo {
            model,
            min_samples: min_samples(&cfg),
            max_samples: cfg.max_samples(),
            device: device_name,
        };
        Ok(RobloxClassifier {
            runner,
            cfg,
            info,
            device,
            pool: threads.map(pool).transpose()?,
        })
    }

    /// Runs the model and returns everything along the way (for golden tests and diagnostics).
    pub fn trace(&self, pcm16k: &[f32]) -> Result<Trace<B>, ModelError> {
        let n = pcm16k.len();
        if n < self.info.min_samples {
            return Err(ModelError::TooShort {
                samples: n,
                min: self.info.min_samples,
            });
        }
        if n > self.info.max_samples {
            return Err(ModelError::TooLong {
                samples: n,
                max: self.info.max_samples,
            });
        }
        let run = || {
            let wav = Tensor::<B, 2>::from_data(TensorData::new(pcm16k.to_vec(), [1, n]), &self.device);
            self.runner.forward(wav)
        };
        Ok(match &self.pool {
            Some(pool) => pool.install(run),
            None => run(),
        })
    }

    pub fn config(&self) -> &ModelConfig {
        &self.cfg
    }
}

impl RobloxClassifier<Cpu> {
    /// Loads the model for the CPU, using `threads` threads.
    pub fn load_cpu(dir: &Path, threads: NonZeroUsize) -> Result<Self, ModelError> {
        Self::build(
            dir,
            Default::default(),
            format!("CPU, {threads} threads"),
            Some(threads),
        )
    }
}

/// Which GPU to run on (wgpu; Vulkan on Linux).
#[cfg(feature = "gpu")]
pub use burn::backend::wgpu::WgpuDevice as GpuDevice;

#[cfg(feature = "gpu")]
impl RobloxClassifier<Gpu> {
    /// Loads the model onto a GPU. `GpuDevice::DiscreteGpu(0)` is the first discrete card (an integrated GPU next
    /// to it is never picked by accident).
    pub fn load_gpu(dir: &Path, device: GpuDevice) -> Result<Self, ModelError> {
        let name = format!("GPU (wgpu, {device:?})");
        Self::build(dir, device, name, None)
    }
}

fn to_array<const N: usize, B: Backend>(t: Tensor<B, 1>) -> Result<[f32; N], ModelError> {
    let v: Vec<f32> = t
        .into_data()
        .to_vec()
        .map_err(|e| ModelError::Failed(format!("{e:?}")))?;
    v.try_into()
        .map_err(|v: Vec<f32>| ModelError::Failed(format!("expected {N} outputs, got {}", v.len())))
}

impl<B: Backend> Classifier for RobloxClassifier<B> {
    fn info(&self) -> &ClassifierInfo {
        &self.info
    }

    fn classify(&mut self, pcm16k: &[f32]) -> Result<RawScores, ModelError> {
        let trace = self.trace(pcm16k)?;
        let labels = to_array::<8, B>(sigmoid(trace.logits))?;
        let languages = to_array::<30, B>(softmax(trace.language_logits, 0))?;
        if labels.iter().chain(languages.iter()).any(|p| !p.is_finite()) {
            return Err(ModelError::Failed("the model produced a non-finite score".into()));
        }
        Ok(RawScores { labels, languages })
    }

    fn set_threads(&mut self, threads: NonZeroUsize) -> Result<(), ModelError> {
        if self.pool.is_some() {
            self.pool = Some(pool(threads)?);
            self.info.device = format!("CPU, {threads} threads");
        }
        Ok(())
    }
}
