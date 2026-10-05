//! The voice library: voices a speech model made from a sample, kept in the event log and given to their model at
//! every start. They are chosen like any voice, as `<model>:<id>`.

use bytes::Bytes;
use pb_models_api::{ClonedVoice, TtsInfo};
use pb_store_api::{Actor, BlobAdded, BlobRole, Event, VoiceRecord, VoiceRemoved};

use super::core::{Core, Phase};
use super::engine::Engine;
use super::error::{EngineError, VoiceError};
use super::library::{decode_staged, media_type};

/// An id made from a name (`Anna (calm)` → `anna-calm`), unlike every id in `taken`.
fn voice_id(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut slug = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = match slug.trim_end_matches('-') {
        "" => "voice".to_owned(),
        s => s.to_owned(),
    };
    (1..)
        .map(|n| if n == 1 { slug.clone() } else { format!("{slug}-{n}") })
        .find(|id| !taken(id))
        .unwrap_or(slug)
}

impl Engine {
    /// The speech models that run, and whether they make voices from samples.
    pub fn speech_models(&self) -> Vec<TtsInfo> {
        self.core.deps.inference.speech_models()
    }

    /// The voice library, oldest first.
    pub fn library_voices(&self) -> Vec<VoiceRecord> {
        let mut out: Vec<VoiceRecord> = self.core.voices.get().values().cloned().collect();
        out.sort_by_key(|v| v.voice_id());
        out
    }

    /// A voice in the library, by `<model>:<id>`.
    pub fn library_voice(&self, voice: &str) -> Option<VoiceRecord> {
        let (model, id) = voice.split_once(':')?;
        self.core.voices.get().get(&(model.to_owned(), id.to_owned())).cloned()
    }

    /// Makes a voice with `model` from an uploaded or recorded file (and what is said in it, which some models need)
    /// and adds it to the library.
    pub async fn add_voice(
        &self,
        staged: std::path::PathBuf,
        ext: Option<String>,
        name: String,
        model: String,
        transcript: Option<String>,
        by: Actor,
    ) -> Result<VoiceRecord, EngineError> {
        let inference = &self.core.deps.inference;
        let (sample, size) = decode_staged(&staged, ext.as_deref(), Ok).await?;
        let transcript = transcript.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        let made = match inference
            .clone_voice(&model, sample.samples, sample.rate, transcript.clone())
            .await
        {
            Ok(m) => m,
            Err(e) => {
                let _ = tokio::fs::remove_file(&staged).await;
                return Err(VoiceError::from(e).into());
            }
        };
        let blobs = &self.core.deps.blobs;
        let original = blobs.put_file(&staged).await?;
        let data = blobs.put(Bytes::from(made.data.clone())).await?;
        // The id is taken here, at once, so two voices added at the same time get different ones.
        let installed = inference.voices();
        let rec = self.core.voices.update(|vs| {
            let id = voice_id(&name, |id| {
                vs.contains_key(&(model.clone(), id.to_owned()))
                    || installed.iter().any(|v| v.model == model && v.id == id)
            });
            let rec = VoiceRecord {
                model: model.clone(),
                id,
                name: name.trim().to_owned(),
                sample: original.hash,
                data: data.hash,
                transcript,
                added_by: by.clone(),
                by,
            };
            vs.insert((rec.model.clone(), rec.id.clone()), rec.clone());
            rec
        });
        let forget = || {
            self.core
                .voices
                .update(|vs| vs.remove(&(rec.model.clone(), rec.id.clone())));
        };
        if let Err(e) = inference.add_voice(&rec.model, &rec.id, made).await {
            forget();
            return Err(VoiceError::from(e).into());
        }
        let events = vec![
            Event::BlobAdded(BlobAdded {
                hash: original.hash,
                size,
                media_type: media_type(ext.as_deref()),
                role: BlobRole::Original,
            }),
            Event::BlobAdded(BlobAdded {
                hash: data.hash,
                size: data.size,
                media_type: "application/octet-stream".into(),
                role: BlobRole::Voice,
            }),
            Event::VoiceSaved(Box::new(rec.clone())),
        ];
        if !self.core.record_durably(events).await {
            forget();
            let _ = inference.remove_voice(&rec.model, &rec.id).await;
            return Err(EngineError::LogHalted);
        }
        Ok(rec)
    }

    /// Renames a voice in the library (its id stays).
    pub async fn rename_voice(&self, voice: &str, name: String, by: Actor) -> Result<VoiceRecord, EngineError> {
        let mut rec = self.library_voice(voice).ok_or(EngineError::NoSuchVoice)?;
        rec.name = name.trim().to_owned();
        rec.by = by;
        if !self
            .core
            .record_durably(vec![Event::VoiceSaved(Box::new(rec.clone()))])
            .await
        {
            return Err(EngineError::LogHalted);
        }
        self.core
            .voices
            .update(|vs| vs.insert((rec.model.clone(), rec.id.clone()), rec.clone()));
        Ok(rec)
    }

    /// Removes a voice from the library (its files stay; lines that used it speak in the voice for their language).
    pub async fn remove_voice(&self, voice: &str, by: Actor) -> Result<(), EngineError> {
        let rec = self.library_voice(voice).ok_or(EngineError::NoSuchVoice)?;
        let removed = VoiceRemoved {
            model: rec.model.clone(),
            id: rec.id.clone(),
            by,
        };
        if !self.core.record_durably(vec![Event::VoiceRemoved(removed)]).await {
            return Err(EngineError::LogHalted);
        }
        self.core
            .voices
            .update(|vs| vs.remove(&(rec.model.clone(), rec.id.clone())));
        if let Err(e) = self.core.deps.inference.remove_voice(&rec.model, &rec.id).await {
            // Its model does not run (then it never had the voice).
            tracing::debug!(voice = %rec.voice_id(), error = %e, "a removed voice was not with its model");
        }
        Ok(())
    }
}

/// Gives every voice in the library to its model (at start; a model still loading gets them when it is ready, one
/// that is not installed leaves them unusable and lines fall back to the voice for their language).
pub(super) async fn register_library(core: std::sync::Arc<Core>) {
    let mut phase = core.phase.subscribe();
    let stopping = async { phase.wait_for(|p| *p != Phase::Running).await.map(drop) };
    let all = async {
        for rec in core.voices.get().values() {
            let data = match core.deps.blobs.get(&rec.data).await {
                Ok(Some(d)) => d.to_vec(),
                Ok(None) => {
                    tracing::warn!(voice = %rec.voice_id(), "a voice's data is missing; it cannot be used");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(voice = %rec.voice_id(), error = %e, "a voice's data could not be read");
                    continue;
                }
            };
            if let Err(e) = core
                .deps
                .inference
                .add_voice(&rec.model, &rec.id, ClonedVoice { data })
                .await
            {
                tracing::warn!(voice = %rec.voice_id(), error = %e, "a voice from the library cannot be used");
            }
        }
    };
    tokio::select! {
        () = all => {}
        _ = stopping => {}
    }
}

#[cfg(test)]
mod tests {
    use super::voice_id;

    #[test]
    fn ids_come_from_names_and_never_repeat() {
        let none = |_: &str| false;
        assert_eq!(voice_id("Anna (calm)", none), "anna-calm");
        assert_eq!(voice_id("  Grüße!  ", none), "grüße");
        assert_eq!(voice_id("???", none), "voice");
        assert_eq!(voice_id("Anna", |id| ["anna", "anna-2"].contains(&id)), "anna-3");
    }
}
