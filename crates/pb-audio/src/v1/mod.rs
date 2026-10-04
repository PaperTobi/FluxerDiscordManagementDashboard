//! Version 1.

mod decode;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

pub use decode::{decode, decode_file};

pub use pb_domain::PLAY_RATE;
/// The loudness voice clips are brought to.
pub const CLIP_LUFS: f64 = -16.0;
/// The highest peak of a prepared clip (−1 dBFS).
pub const CLIP_PEAK_DBFS: f32 = -1.0;
/// Fade in and out of a prepared clip, so it never clicks.
pub const CLIP_FADE_MS: u32 = 5;

/// Mono audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    /// In [-1, 1].
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.rate.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    #[error("not a known audio format: {0}")]
    Unsupported(String),
    #[error("the audio could not be decoded: {0}")]
    Decode(String),
    #[error("there is no sound in it")]
    Empty,
    #[error("resampling failed: {0}")]
    Resample(String),
}

pub fn from_i16(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| f32::from(s) / 32768.0).collect()
}

pub fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&x| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
        .collect()
}

/// Resamples mono audio (the whole buffer; the resampler's delay is removed).
pub fn resample(samples: &[f32], from: u32, to: u32) -> Result<Vec<f32>, AudioError> {
    if from == to || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let err = |e: &dyn std::fmt::Display| AudioError::Resample(e.to_string());
    let mut r = Fft::<f32>::new(from as usize, to as usize, 1024, 1, FixedSync::Input).map_err(|e| err(&e))?;
    let out_len = r.process_all_needed_output_len(samples.len());
    let mut out = vec![0.0f32; out_len];
    let input = InterleavedSlice::new(samples, 1, samples.len()).map_err(|e| err(&e))?;
    let mut output = InterleavedSlice::new_mut(&mut out[..], 1, out_len).map_err(|e| err(&e))?;
    let (_, produced) = r
        .process_all_into_buffer(&input, &mut output, samples.len(), None)
        .map_err(|e| err(&e))?;
    out.truncate(produced);
    Ok(out)
}

/// 16-bit mono WAV. RIFF sizes are 32-bit, so a WAV holds at most 4 GiB (about 12 h at 48 kHz); longer audio is cut
/// there.
pub fn wav16(samples: &[i16], rate: u32) -> Vec<u8> {
    let max = (u32::MAX as usize - 36) / 2;
    let samples = &samples[..samples.len().min(max)];
    let data = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // integer PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&rate.saturating_mul(2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Where things are in a WAV file (RIFF/WAVE): enough to read 16-bit PCM exactly or to know the length without
/// decoding. Anything else is for [`decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavLayout {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    /// 1 = integer PCM, 3 = float (`WAVE_FORMAT_EXTENSIBLE` resolved to its sub-format).
    pub format: u16,
    /// The bytes of the samples.
    pub data: std::ops::Range<usize>,
}

impl WavLayout {
    pub fn frames(&self) -> u64 {
        let frame = usize::from(self.bits / 8).max(1) * usize::from(self.channels.max(1));
        (self.data.len() / frame) as u64
    }

    pub fn millis(&self) -> u64 {
        self.frames() * 1000 / u64::from(self.rate.max(1))
    }

    /// The interleaved samples of 16-bit integer PCM (`None` for other formats).
    pub fn i16_samples(&self, bytes: &[u8]) -> Option<Vec<i16>> {
        if self.format != 1 || self.bits != 16 {
            return None;
        }
        let data = bytes.get(self.data.clone())?;
        Some(data.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads a WAV file's layout (`None` = not a WAV file this understands).
pub fn wav_layout(bytes: &[u8]) -> Option<WavLayout> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while pos.checked_add(8)? <= bytes.len() {
        let id = bytes.get(pos..pos + 4)?;
        let size = usize::try_from(u32_at(bytes, pos + 4)?).ok()?;
        let body = pos + 8;
        if id == b"fmt " {
            let b = bytes.get(body..body.checked_add(size)?.min(bytes.len()))?;
            let mut format = u16_at(b, 0)?;
            if format == 0xFFFE {
                format = u16_at(b, 24)?;
            }
            fmt = Some((format, u16_at(b, 2)?, u32_at(b, 4)?, u16_at(b, 14)?));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt?;
            // A writer that streamed may leave the size 0 or too large: the samples then run to the end.
            let end = if size == 0 {
                bytes.len()
            } else {
                body.saturating_add(size).min(bytes.len())
            };
            return Some(WavLayout {
                rate,
                channels,
                bits,
                format,
                data: body..end,
            });
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// Root mean square in dBFS (−120 for silence).
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let ms = samples.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / samples.len() as f64;
    let db = (10.0 * ms.max(1e-12).log10()) as f32;
    db.max(-120.0)
}

/// Integrated loudness (EBU R128) in LUFS; `None` for silence or audio shorter than one 400 ms block.
pub fn loudness_lufs(samples: &[f32], rate: u32) -> Option<f64> {
    let mut m = ebur128::EbuR128::new(1, rate, ebur128::Mode::I).ok()?;
    m.add_frames_f32(samples).ok()?;
    m.loudness_global().ok().filter(|l| l.is_finite())
}

/// Multiplies by a gain in dB.
pub fn gain_db(samples: &mut [f32], db: f32) {
    let g = 10f32.powf(db / 20.0);
    for x in samples {
        *x *= g;
    }
}

/// Scales the whole signal down so its peak is at most `ceiling_dbfs` (no distortion; quieter only when needed).
pub fn limit_peak(samples: &mut [f32], ceiling_dbfs: f32) {
    let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let ceiling = 10f32.powf(ceiling_dbfs / 20.0);
    if peak > ceiling {
        let g = ceiling / peak;
        for x in samples {
            *x *= g;
        }
    }
}

/// Linear fades in and out (each at most half the signal).
pub fn fade(samples: &mut [f32], rate: u32, ms: u32) {
    let n = ((u64::from(rate) * u64::from(ms) / 1000) as usize).min(samples.len() / 2);
    for i in 0..n {
        let g = i as f32 / n as f32;
        samples[i] *= g;
        let j = samples.len() - 1 - i;
        samples[j] *= g;
    }
}

/// A voice clip ready to play: 48 kHz, −16 LUFS (quiet clips are brought up only as far as the peak allows),
/// peak ≤ −1 dBFS, short fades.
pub fn prepare_clip(pcm: &Pcm) -> Result<Vec<i16>, AudioError> {
    if pcm.samples.iter().all(|&x| x.abs() < 1e-4) {
        return Err(AudioError::Empty);
    }
    let mut x = resample(&pcm.samples, pcm.rate, PLAY_RATE)?;
    if let Some(l) = loudness_lufs(&x, PLAY_RATE) {
        gain_db(&mut x, (CLIP_LUFS - l) as f32);
    }
    limit_peak(&mut x, CLIP_PEAK_DBFS);
    fade(&mut x, PLAY_RATE, CLIP_FADE_MS);
    Ok(to_i16(&x))
}

#[cfg(test)]
mod tests {

    #[test]
    fn wav16_round_trips_and_decodes() {
        let samples: Vec<i16> = (0..4800).map(|i| ((i % 200) as i16 - 100) * 300).collect();
        let bytes = wav16(&samples, 48_000);
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        let l = wav_layout(&bytes).expect("a WAV file");
        assert_eq!((l.rate, l.channels, l.bits, l.format), (48_000, 1, 16, 1));
        assert_eq!(l.millis(), 100);
        assert_eq!(l.i16_samples(&bytes).as_deref(), Some(&samples[..]));
        // The general decoder reads it the same.
        let pcm = decode(&bytes, Some("wav")).expect("symphonia decodes it");
        assert_eq!(pcm.rate, 48_000);
        assert_eq!(to_i16(&pcm.samples), samples);
    }

    #[test]
    fn wav_layout_skips_chunks_and_rejects_others() {
        let mut bytes = wav16(&[1, 2, 3], 16_000);
        // An odd-sized LIST chunk before `data` (padded to an even length).
        let list = [b"LIST".as_slice(), &3u32.to_le_bytes(), b"abc\0"].concat();
        bytes.splice(36..36, list);
        let l = wav_layout(&bytes).expect("a WAV file");
        assert_eq!(l.i16_samples(&bytes), Some(vec![1, 2, 3]));
        assert_eq!(wav_layout(b"OggS...."), None);
        assert_eq!(wav_layout(b""), None);
    }

    use super::*;

    /// The highest absolute sample in dBFS (−120 for silence).
    fn peak_dbfs(samples: &[f32]) -> f32 {
        let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        (20.0 * peak.max(1e-6).log10()).max(-120.0)
    }

    fn tone(rate: u32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / rate as f32).sin() * amp)
            .collect()
    }

    #[test]
    fn resample_keeps_length_and_pitch() {
        let x = tone(16_000, 1.0, 0.5);
        let y = resample(&x, 16_000, 48_000).unwrap();
        assert!((y.len() as i64 - 48_000).abs() < 200, "{}", y.len());
        let z = resample(&y, 48_000, 16_000).unwrap();
        assert!((rms_dbfs(&z[2000..14000]) - rms_dbfs(&x[2000..14000])).abs() < 0.2);
    }

    #[test]
    fn prepared_clips_are_loud_enough_and_never_clip() {
        let quiet = Pcm {
            rate: 22_050,
            samples: tone(22_050, 2.0, 0.02),
        };
        let out = from_i16(&prepare_clip(&quiet).unwrap());
        let l = loudness_lufs(&out, PLAY_RATE).unwrap();
        assert!(l > -17.0 && peak_dbfs(&out) <= -0.99, "{l} {}", peak_dbfs(&out));
        let loud = Pcm {
            rate: 48_000,
            samples: tone(48_000, 2.0, 1.0),
        };
        let out = from_i16(&prepare_clip(&loud).unwrap());
        assert!(peak_dbfs(&out) <= -0.99);
        assert_eq!(out[0], 0.0, "fades in");
        assert!(matches!(
            prepare_clip(&Pcm {
                rate: 48_000,
                samples: vec![0.0; 4800]
            }),
            Err(AudioError::Empty)
        ));
    }

    #[test]
    fn wav_round_trip() {
        let pcm: Vec<i16> = (0..1000).map(|i| (i * 13 % 2000 - 1000) as i16).collect();
        let bytes = wav16(&pcm, 16_000);
        let back = decode(&bytes, Some("wav")).unwrap();
        assert_eq!(back.rate, 16_000);
        assert_eq!(to_i16(&back.samples), pcm);
    }
}
