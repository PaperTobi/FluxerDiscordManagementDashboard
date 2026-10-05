//! The Roblox voice-safety classifier v3 (`Roblox/voice-safety-classifier-v3`, Apache-2.0) written in Burn and loaded
//! from the official `model.safetensors`. Computes the same numbers as Roblox's `inference.py` for one clip
//! (checked by the golden tests against the PyTorch reference). Runs on the CPU with Burn's pure-Rust backend, or
//! with the `gpu` feature on a GPU through wgpu (Vulkan).

mod config;
mod frontend;
#[cfg(feature = "gpu")]
mod gpu;
mod mask;
pub mod model;
mod ops;

use std::num::NonZeroUsize;
use std::path::Path;

use burn::module::Module;
use burn::tensor::Tensor;
use burn::tensor::activation::{sigmoid, softmax};
use burn_store::{ModuleSnapshot, PyTorchToBurnAdapter, SafetensorsStore};
use pb_models_api::{Classifier, ClassifierInfo, ModelError, RawScores};

pub use config::{ConfigError, ModelConfig};
pub use model::{Model, Runner, Trace};
pub use ops::{Ops, Then};

/// Burn's pure-Rust CPU backend.
pub type Cpu = burn::backend::Flex;
#[cfg(feature = "gpu")]
pub type Gpu = burn::backend::Wgpu;

/// Files in the model directory.
pub const CONFIG_FILE: &str = "config.json";
pub const WEIGHTS_FILE: &str = "model.safetensors";
pub const REVISION_FILE: &str = "REVISION";

/// The classifier, bound to one device. CPU work runs on its own thread pool, sized by `set_threads`.
pub struct RobloxClassifier<B: Ops> {
    runner: Runner<B>,
    cfg: ModelConfig,
    info: ClassifierInfo,
    pool: Option<rayon::ThreadPool>,
}

impl<B: Ops> std::fmt::Debug for RobloxClassifier<B> {
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
pub fn load_runner<B: Ops>(dir: &Path, device: &B::Device) -> Result<(Runner<B>, ModelConfig), ModelError> {
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
    Ok((Runner::new(model, &cfg)?, cfg))
}

impl<B: Ops> RobloxClassifier<B> {
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
        let run = || self.runner.forward(pcm16k);
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
        // Without a usable GPU (no Vulkan driver, no device) wgpu panics while setting up: an error, not a crash.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut clf = Self::build(dir, device, name, None)?;
            clf.warm_up()?;
            Ok(clf)
        }))
        .unwrap_or_else(|panic| {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(ModelError::Load(format!("no usable GPU (Vulkan): {why}")))
        })
    }

    /// Classifies a few silent clips, so that the GPU compiles its kernels now rather than during the first real
    /// clips. Some kernels come in variants for lengths that are or are not multiples of 2 and 4; consecutive frame
    /// counts cover them.
    fn warm_up(&mut self) -> Result<(), ModelError> {
        let hop = self.cfg.hop_length;
        let base = (self.cfg.sample_rate as usize).max(self.info.min_samples);
        for extra in 0..WARM_UP_LENGTHS {
            self.classify(&vec![0.0; base + extra * hop])?;
        }
        Ok(())
    }
}

/// Clip lengths [`RobloxClassifier::warm_up`] runs (one frame apart).
#[cfg(feature = "gpu")]
const WARM_UP_LENGTHS: usize = 4;

impl<B: Ops> Classifier for RobloxClassifier<B> {
    fn info(&self) -> &ClassifierInfo {
        &self.info
    }

    fn classify(&mut self, pcm16k: &[f32]) -> Result<RawScores, ModelError> {
        let trace = self.trace(pcm16k)?;
        // One read back for both outputs: each read waits for the device.
        let scores = Tensor::cat(vec![sigmoid(trace.logits), softmax(trace.language_logits, 0)], 0);
        let scores: [f32; 38] = scores
            .into_data()
            .to_vec::<f32>()
            .map_err(|e| ModelError::Failed(format!("{e:?}")))?
            .try_into()
            .map_err(|v: Vec<f32>| ModelError::Failed(format!("expected 38 outputs, got {}", v.len())))?;
        let labels: [f32; 8] = std::array::from_fn(|i| scores[i]);
        let languages: [f32; 30] = std::array::from_fn(|i| scores[8 + i]);
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
