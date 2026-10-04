//! Every format a browser recording or an upload is likely to be in decodes to the same tone.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_audio::{decode, rms_dbfs};

fn check(name: &str) {
    check_with(name, 0.03);
}

fn check_with(name: &str, tolerance_s: f64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap();
    let ext = path.extension().and_then(|e| e.to_str());
    let pcm = decode(&bytes, ext).unwrap_or_else(|e| panic!("{name}: {e}"));
    let secs = pcm.seconds();
    assert!((secs - 1.0).abs() < tolerance_s, "{name}: {secs} s");
    // As loud as the WAV source (lossy codecs within a dB): ffmpeg's sine is at 1/8, halved, and −3 dB per channel
    // from its mono-to-stereo upmix.
    let mid = &pcm.samples[pcm.samples.len() / 4..pcm.samples.len() * 3 / 4];
    let rms = rms_dbfs(mid);
    assert!((rms + 30.10).abs() < 1.0, "{name}: {rms} dBFS");
    // The pitch, from the zero crossings in the middle half.
    let crossings = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    let hz = crossings as f64 / 2.0 / (mid.len() as f64 / f64::from(pcm.rate));
    assert!((hz - 440.0).abs() < 4.0, "{name}: {hz} Hz");
}

#[test]
fn wav() {
    check("tone.wav");
}

#[test]
fn flac() {
    check("tone.flac");
}

#[test]
fn mp3() {
    check("tone.mp3");
}

#[test]
fn ogg_vorbis() {
    check("tone.ogg");
}

#[test]
fn ogg_opus() {
    check("tone.opus");
}

#[test]
fn webm_opus() {
    check("tone.webm");
}

/// symphonia 0.6.1 does not apply MP4 edit lists yet, so AAC's 1024 priming samples and the end padding stay
/// (about 45 ms of near-silence; harmless for a voice clip).
#[test]
fn m4a_aac() {
    check_with("tone.m4a", 0.05);
}

#[test]
fn garbage_is_an_error() {
    assert!(decode(b"definitely not audio", None).is_err());
}
