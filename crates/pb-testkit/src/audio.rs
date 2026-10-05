//! Reading WAV fixtures, resampling and comparing audio in tests.

use std::path::Path;

use anyhow::{Context, Result, bail};

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

/// Resample mono 16-bit audio (pb-audio's resampler, as the bot uses).
pub fn resample(input: &[i16], from: u32, to: u32) -> Result<Vec<i16>> {
    Ok(pb_audio::to_i16(&pb_audio::resample(
        &pb_audio::from_i16(input),
        from,
        to,
    )?))
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
