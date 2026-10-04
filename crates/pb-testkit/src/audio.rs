//! Reading WAV fixtures, resampling and comparing audio in tests.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// Mono 16-bit samples and their rate.
pub fn read_wav(path: &Path) -> Result<(Vec<i16>, u32)> {
    let bytes = std::fs::read(path).with_context(|| format!("opening {}", path.display()))?;
    let Some(layout) = pb_audio::wav_layout(&bytes) else {
        bail!("{}: not a WAV file", path.display());
    };
    let Some(samples) = layout.i16_samples(&bytes) else {
        bail!("{}: expected 16-bit PCM", path.display());
    };
    let channels = usize::from(layout.channels.max(1));
    let mono = samples.chunks(channels).map(|c| c[0]).collect();
    Ok((mono, layout.rate))
}

/// Resample mono 16-bit audio (whole buffer, resampler delay trimmed).
pub fn resample(input: &[i16], from: u32, to: u32) -> Result<Vec<i16>> {
    if from == to {
        return Ok(input.to_vec());
    }
    let floats: Vec<f32> = input.iter().map(|&s| f32::from(s) / 32768.0).collect();
    let mut resampler = Fft::<f32>::new(from as usize, to as usize, 1024, 1, FixedSync::Input)?;
    let out_len = resampler.process_all_needed_output_len(floats.len());
    let mut out = vec![0.0f32; out_len];
    let input_buf = InterleavedSlice::new(&floats[..], 1, floats.len())?;
    let mut output_buf = InterleavedSlice::new_mut(&mut out[..], 1, out_len)?;
    let (_, produced) = resampler.process_all_into_buffer(&input_buf, &mut output_buf, floats.len(), None)?;
    out.truncate(produced);
    Ok(out
        .iter()
        .map(|&x| (x * 32767.0).clamp(-32768.0, 32767.0) as i16)
        .collect())
}

/// Loudness envelope: RMS per `window` samples.
pub fn envelope(samples: &[i16], window: usize) -> Vec<f32> {
    samples
        .chunks(window)
        .map(|c| (c.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / c.len() as f64).sqrt() as f32)
        .collect()
}

/// Best normalised cross-correlation of `b` against `a` over lags `0..=max_lag` (b delayed relative to a).
pub fn best_correlation(a: &[f32], b: &[f32], max_lag: usize) -> (f32, usize) {
    let mut best = (f32::MIN, 0);
    for lag in 0..=max_lag.min(b.len().saturating_sub(1)) {
        let n = a.len().min(b.len() - lag);
        if n < 10 {
            break;
        }
        let (x, y) = (&a[..n], &b[lag..lag + n]);
        let mx = x.iter().sum::<f32>() / n as f32;
        let my = y.iter().sum::<f32>() / n as f32;
        let (mut sxy, mut sxx, mut syy) = (0.0f32, 0.0f32, 0.0f32);
        for (p, q) in x.iter().zip(y) {
            sxy += (p - mx) * (q - my);
            sxx += (p - mx).powi(2);
            syy += (q - my).powi(2);
        }
        let r = if sxx > 0.0 && syy > 0.0 {
            sxy / (sxx.sqrt() * syy.sqrt())
        } else {
            0.0
        };
        if r > best.0 {
            best = (r, lag);
        }
    }
    best
}
