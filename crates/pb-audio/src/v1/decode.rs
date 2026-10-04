//! Any common audio file → mono PCM at its own rate.

use std::io::Cursor;

use opus_decoder::OpusDecoder;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::{AudioError, Pcm};

/// Appends interleaved `frames` mixed down to mono.
fn mix_into(out: &mut Vec<f32>, interleaved: &[f32], channels: usize) {
    let ch = channels.max(1);
    let scale = 1.0 / ch as f32;
    out.extend(interleaved.chunks_exact(ch).map(|f| f.iter().sum::<f32>() * scale));
}

/// Decodes `bytes` (WAV, FLAC, MP3, Ogg Vorbis/Opus, WebM/Matroska Opus or Vorbis, MP4/M4A AAC/ALAC, CAF, AIFF …);
/// `ext` (a file extension) helps guess the format.
pub fn decode(bytes: &[u8], ext: Option<&str>) -> Result<Pcm, AudioError> {
    decode_source(Box::new(Cursor::new(bytes.to_vec())), ext)
}

/// [`decode`] reading a file (it is not loaded into memory as a whole).
pub fn decode_file(path: &std::path::Path, ext: Option<&str>) -> Result<Pcm, AudioError> {
    let file = std::fs::File::open(path).map_err(|e| AudioError::Decode(e.to_string()))?;
    decode_source(Box::new(file), ext)
}

fn decode_source(source: Box<dyn symphonia::core::io::MediaSource>, ext: Option<&str>) -> Result<Pcm, AudioError> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| AudioError::Unsupported(e.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| AudioError::Unsupported("no audio track".into()))?;
    let track_id = track.id;
    let (track_delay, track_padding) = (track.delay, track.padding);
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .cloned()
        .ok_or_else(|| AudioError::Unsupported("no audio parameters".into()))?;
    let mut samples = Vec::new();
    if params.codec == CODEC_ID_OPUS {
        let channels = params.channels.as_ref().map_or(1, |c| c.count()).clamp(1, 2);
        let mut dec = OpusDecoder::new(48_000, channels).map_err(|e| AudioError::Decode(format!("{e:?}")))?;
        let mut buf = vec![0.0f32; 5760 * channels];
        let mut trimmed = false;
        loop {
            let packet = match format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(Error::ResetRequired) => break,
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };
            if packet.track_id != track_id {
                continue;
            }
            let n = dec
                .decode_float(&packet.data, &mut buf, false)
                .map_err(|e| AudioError::Decode(format!("{e:?}")))?;
            let skip = usize::try_from(packet.trim_start.get()).unwrap_or(0).min(n);
            let cut = usize::try_from(packet.trim_end.get()).unwrap_or(0).min(n - skip);
            trimmed |= skip > 0 || cut > 0;
            mix_into(&mut samples, &buf[skip * channels..(n - cut) * channels], channels);
        }
        if !trimmed {
            // The container did not trim per packet (WebM): drop the encoder's pre-skip (the track delay, or the
            // OpusHead pre-skip field) and the end padding.
            let pre_skip = track_delay.map(|d| d as usize).or_else(|| {
                let head = params.extra_data.as_deref()?;
                (head.len() >= 12 && head.starts_with(b"OpusHead"))
                    .then(|| usize::from(u16::from_le_bytes([head[10], head[11]])))
            });
            samples.drain(..pre_skip.unwrap_or(0).min(samples.len()));
            let pad = track_padding.map_or(0, |p| p as usize).min(samples.len());
            samples.truncate(samples.len() - pad);
        }
        return if samples.is_empty() {
            Err(AudioError::Empty)
        } else {
            Ok(Pcm { rate: 48_000, samples })
        };
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| AudioError::Unsupported(e.to_string()))?;
    let mut rate = params.sample_rate.unwrap_or(0);
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                rate = buf.spec().rate();
                let channels = buf.spec().channels().count().max(1);
                interleaved.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut interleaved);
                // Encoder delay and padding (gapless trimming).
                let n = interleaved.len() / channels;
                let skip = usize::try_from(packet.trim_start.get()).unwrap_or(0).min(n);
                let cut = usize::try_from(packet.trim_end.get()).unwrap_or(0).min(n - skip);
                mix_into(
                    &mut samples,
                    &interleaved[skip * channels..(n - cut) * channels],
                    channels,
                );
            }
            // A damaged packet is skipped, like players do.
            Err(Error::DecodeError(_)) => {}
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        }
    }
    if samples.is_empty() || rate == 0 {
        return Err(AudioError::Empty);
    }
    Ok(Pcm { rate, samples })
}
