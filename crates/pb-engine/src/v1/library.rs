//! The clip library and recordings, for the web UI.

use std::sync::Arc;

use bytes::Bytes;
use pb_domain::{BlobHash, ClfLang, GuildId, Lang, SentenceId, UserId};
use pb_infer::Priority;
use pb_store_api::{Actor, BlobAdded, BlobDeleted, BlobRole, ClipRecord, ClipRemoved, Event};

use super::engine::Engine;
use super::error::EngineError;

/// What "Say now" says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SayWhat {
    /// A text, spoken in `lang` (else the person's language).
    Text { text: String, lang: Option<Lang> },
    /// One of the "say" voice lines (`say.<preset>`): its clips or texts, like any voice line.
    Preset(String),
}

/// A media type for a file name's extension.
fn media_type(ext: Option<&str>) -> String {
    match ext.map(str::to_ascii_lowercase).as_deref() {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("webm") => "audio/webm",
        Some("m4a" | "mp4" | "aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("caf") => "audio/x-caf",
        Some("mkv" | "mka") => "audio/x-matroska",
        _ => "application/octet-stream",
    }
    .to_owned()
}

impl Engine {
    /// Where an upload is written while it arrives (then handed to [`Engine::add_clip`]).
    pub fn upload_dir(&self) -> &std::path::Path {
        self.core.deps.blobs.staging_dir()
    }

    /// Adds a clip: keeps the file as it arrived, stores a prepared render (48 kHz, −16 LUFS, short fades) and checks
    /// it with the classifier (a clip that sounds like a violation is marked; the language heard is noted).
    pub async fn add_clip(
        &self,
        staged: std::path::PathBuf,
        name: String,
        ext: Option<String>,
        lang: Option<Lang>,
        by: Actor,
    ) -> Result<ClipRecord, EngineError> {
        // Decoded off the async threads; a file that is not audio is removed again.
        let (decoded, size) = {
            let (path, e) = (staged.clone(), ext.clone());
            tokio::task::spawn_blocking(move || -> Result<(Vec<i16>, u64), EngineError> {
                let done = pb_audio::decode_file(&path, e.as_deref())
                    .and_then(|pcm| pb_audio::prepare_clip(&pcm))
                    .map_err(EngineError::Unreadable)
                    .and_then(|pcm| {
                        Ok((
                            pcm,
                            std::fs::metadata(&path).map_err(pb_store_api::StoreError::from)?.len(),
                        ))
                    });
                if done.is_err() {
                    let _ = std::fs::remove_file(&path);
                }
                done
            })
            .await
            // The decoder crashed on this file.
            .map_err(|e| EngineError::Unreadable(pb_audio::AudioError::Decode(e.to_string())))??
        };
        let blobs = &self.core.deps.blobs;
        let original = blobs.put_file(&staged).await?;
        let wav = pb_audio::wav16(&decoded, pb_audio::PLAY_RATE);
        let render = blobs.put(Bytes::from(wav)).await?;
        let dur_ms = u32::try_from(decoded.len() as u64 * 1000 / u64::from(pb_audio::PLAY_RATE)).unwrap_or(u32::MAX);
        let (self_check, heard_language) = self.check_clip(&decoded).await;
        let rec = ClipRecord {
            render: render.hash,
            original: original.hash,
            name,
            lang,
            transcript: None,
            dur_ms,
            self_check,
            heard_language,
            added_by: by.clone(),
            by,
        };
        let events = vec![
            Event::BlobAdded(BlobAdded {
                hash: original.hash,
                size,
                media_type: media_type(ext.as_deref()),
                role: BlobRole::Original,
            }),
            Event::BlobAdded(BlobAdded {
                hash: render.hash,
                size: render.size,
                media_type: "audio/wav".into(),
                role: BlobRole::Render,
            }),
            Event::ClipSaved(Box::new(rec.clone())),
        ];
        if !self.core.record(events).await {
            return Err(EngineError::LogHalted);
        }
        self.core.put_clip(rec.clone());
        Ok(rec)
    }

    /// The classifier's scores and language for a prepared clip (none when the classifier is not available).
    async fn check_clip(&self, pcm48: &[i16]) -> (Option<[f32; 8]>, Option<ClfLang>) {
        let floats = pb_audio::from_i16(pcm48);
        let Ok(at16) = pb_audio::resample(&floats, pb_audio::PLAY_RATE, 16_000) else {
            return (None, None);
        };
        match self
            .core
            .deps
            .inference
            .classify(Arc::from(at16), Priority::Check)
            .await
        {
            Ok(s) => {
                let lang = s
                    .raw
                    .languages
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .and_then(|(i, _)| ClfLang::ALL.get(i).copied());
                (Some(s.raw.labels), lang)
            }
            Err(e) => {
                tracing::warn!(error = %e, "a clip could not be checked");
                (None, None)
            }
        }
    }

    /// Renames a clip or changes its language or transcript.
    pub async fn update_clip(
        &self,
        render: BlobHash,
        name: String,
        lang: Option<Lang>,
        transcript: Option<String>,
        by: Actor,
    ) -> Result<ClipRecord, EngineError> {
        let mut rec = self.core.clip(&render).ok_or(EngineError::NoSuchClip)?;
        rec.name = name;
        rec.lang = lang;
        rec.transcript = transcript.filter(|t| !t.trim().is_empty());
        rec.by = by;
        if !self.core.record(vec![Event::ClipSaved(Box::new(rec.clone()))]).await {
            return Err(EngineError::LogHalted);
        }
        self.core.put_clip(rec.clone());
        Ok(rec)
    }

    /// Removes a clip from the library (its files stay; voice lines that use it skip it).
    pub async fn remove_clip(&self, render: BlobHash, by: Actor) -> Result<(), EngineError> {
        if self.core.clip(&render).is_none() {
            return Err(EngineError::NoSuchClip);
        }
        if !self
            .core
            .record(vec![Event::ClipRemoved(ClipRemoved { render, by })])
            .await
        {
            return Err(EngineError::LogHalted);
        }
        self.core.remove_clip(&render);
        Ok(())
    }

    /// A clip in the library.
    pub fn clip(&self, render: &BlobHash) -> Option<ClipRecord> {
        self.core.clip(render)
    }

    /// Deletes a sentence's recording (on request); the sentence and that it had a recording stay.
    pub async fn delete_recording(
        &self,
        sentence: SentenceId,
        by: Actor,
        reason: Option<String>,
    ) -> Result<(), EngineError> {
        let row = self
            .core
            .deps
            .index
            .sentence(sentence)
            .await?
            .ok_or(EngineError::NoSuchSentence)?;
        let hash = row.record.audio.ok_or(EngineError::NoRecording)?;
        self.core.deps.blobs.delete(&hash).await?;
        if !self
            .core
            .record(vec![Event::BlobDeleted(BlobDeleted { hash, by, reason })])
            .await
        {
            return Err(EngineError::LogHalted);
        }
        Ok(())
    }
}

impl Engine {
    /// "Say now" for a person: in the call they are in, to the audience set for them — a text (in `lang`, else their
    /// language) or one of the "say" voice lines.
    pub async fn say_to(
        &self,
        guild: GuildId,
        user: UserId,
        what: SayWhat,
        by: Actor,
    ) -> Result<pb_store_api::PlayRecord, EngineError> {
        let channel = self
            .core
            .voice()
            .of_user(user)
            .find(|v| v.guild == guild)
            .map(|v| v.channel)
            .ok_or(EngineError::NotInCall)?;
        let eff = self.core.settings.current().effective(Some(guild), Some(user));
        let audience = pb_domain::Audience::from(eff.audience.value);
        let (line, text) = match what {
            SayWhat::Preset(preset) => (pb_voicelines::Line::Say { preset }, None),
            SayWhat::Text { text, lang } => {
                let lang = match lang {
                    Some(l) => l,
                    None => match &eff.voice_language.value {
                        pb_settings::VoiceLang::Fixed(l) => l.clone(),
                        pb_settings::VoiceLang::Auto => eff
                            .fallback_languages
                            .value
                            .first()
                            .cloned()
                            .ok_or(EngineError::NoLanguage)?,
                    },
                };
                (pb_voicelines::Line::Say { preset: "now".into() }, Some((lang, text)))
            }
        };
        self.say(guild, channel, Some(user), line, text, audience, by).await
    }

    /// Whether a blob is one of the clips shipped with the bot.
    pub fn shipped_clip(&self, h: &BlobHash) -> bool {
        self.core.deps.shipped_clips.iter().any(|s| s.hash == *h)
    }
}

impl Engine {
    /// People in a community whose name starts with `query` (from Fluxer, up to `limit`).
    pub async fn search_members(
        &self,
        guild: GuildId,
        query: &str,
        limit: u32,
    ) -> Result<Vec<pb_live_proto::Who>, EngineError> {
        let ctl = self.core.ctl().ok_or(EngineError::NotConnected)?;
        let found = ctl.search_members(guild, query, limit).await?;
        Ok(found
            .into_iter()
            .filter(|m| !m.user.as_ref().is_some_and(|u| u.bot))
            .map(|m| pb_live_proto::Who {
                user: m.id,
                name: m.shown().to_owned(),
                avatar: self
                    .core
                    .avatar_url(m.id, m.user.as_ref().and_then(|u| u.avatar.as_deref())),
            })
            .collect())
    }
}
