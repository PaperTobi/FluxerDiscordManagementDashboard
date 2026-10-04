//! What to say and how it sounds: a voice line resolved for the person (their languages, their recorded name, the
//! community's and their own lines), planned, and rendered to 48 kHz audio (speech from the text-to-speech thread,
//! clips from the blob store, both cached).

use std::sync::Arc;

use pb_domain::{BlobHash, ClfLang, GuildId, Lang, UserId};
use pb_i18n::{Locale, label_name};
use pb_infer::SpeakPriority;
use pb_models_api::{SpeakOpts, VoiceInfo};
use pb_settings::{SettingsTree, VoiceLang};
use pb_voicelines::{ClipInfo, ClipLang, Field, Fields, Line, Part, Resolution, ResolveCtx, plan, resolve};

use super::core::Core;

/// Audio ready to play, and what it says.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub pcm: Vec<i16>,
    pub text: Option<String>,
    pub lang: Option<Lang>,
    pub clips: Vec<BlobHash>,
    pub line: Option<String>,
}

impl Rendered {
    /// The audio as a WAV file (16-bit mono at the playback rate).
    pub fn wav(&self) -> Vec<u8> {
        pb_audio::wav16(&self.pcm, pb_audio::PLAY_RATE)
    }
}

/// The `Lang` of a classifier language.
pub fn lang_of(l: ClfLang) -> Option<Lang> {
    if l == ClfLang::Other {
        None
    } else {
        l.code().parse().ok()
    }
}

/// The installed voice for a language: the one chosen in the settings, else the first installed one for that
/// language (medium quality first: the old bot's default `en_US-lessac-medium`).
pub fn voice_for(
    voices: &[VoiceInfo],
    chosen: &std::collections::BTreeMap<Lang, String>,
    lang: &Lang,
) -> Option<String> {
    let installed = |id: &str| voices.iter().any(|v| v.id == id);
    if let Some(v) = chosen
        .get(lang)
        .or_else(|| chosen.get(&lang.base()))
        .filter(|v| installed(v))
    {
        return Some(v.clone());
    }
    let matches = |v: &&VoiceInfo| {
        let code = v.language.replace('-', "_").to_ascii_lowercase();
        let want = lang.to_string().replace('-', "_").to_ascii_lowercase();
        code == want || code.split('_').next() == Some(lang.language())
    };
    let mut fitting: Vec<&VoiceInfo> = voices.iter().filter(matches).collect();
    fitting.sort_by_key(|v| (v.quality != "medium", v.quality != "high", v.id.clone()));
    fitting.first().map(|v| v.id.clone())
}

/// The person's languages, in order.
pub fn languages(tree: &SettingsTree, guild: GuildId, person: Option<UserId>, heard: Option<ClfLang>) -> Vec<Lang> {
    let eff = tree.effective(Some(guild), person);
    let mut out: Vec<Lang> = Vec::new();
    match &eff.voice_language.value {
        VoiceLang::Fixed(l) => out.push(l.clone()),
        VoiceLang::Auto => out.extend(heard.and_then(lang_of)),
    }
    for l in &eff.fallback_languages.value {
        if !out.contains(l) {
            out.push(l.clone());
        }
    }
    if out.is_empty() {
        out.push(english());
    }
    out
}

/// English (the language of the shipped clips and the last resort).
pub fn english() -> Lang {
    "en".parse().unwrap_or_else(|_| unreachable!("en is a language tag"))
}

fn clip_info(core: &Core) -> impl Fn(&BlobHash) -> Option<ClipInfo> + '_ {
    move |h| {
        if let Some(c) = core.clip(h) {
            return Some(ClipInfo {
                lang: c.lang.map_or(ClipLang::Unknown, ClipLang::Speech),
            });
        }
        core.deps.shipped_clips.iter().any(|s| s.hash == *h).then(|| ClipInfo {
            lang: ClipLang::Speech(english()),
        })
    }
}

/// Resolves a line for a person (or the community when `person` is `None`).
pub fn resolve_line(core: &Core, guild: GuildId, person: Option<UserId>, line: &Line, langs: &[Lang]) -> Resolution {
    let tree = core.settings.current();
    let eff = tree.effective(Some(guild), person);
    let voices = core.deps.inference.voices();
    let chosen = eff.tts_voices.value.clone();
    let has_voice = |l: &Lang| voice_for(&voices, &chosen, l).is_some();
    let shipped: Vec<BlobHash> = core.deps.shipped_clips.iter().map(|s| s.hash).collect();
    let info = clip_info(core);
    let ctx = ResolveCtx {
        languages: langs,
        slots: tree.scoped_slots(guild, person),
        clip: &info,
        has_voice: &has_voice,
        fallback_clips: &shipped,
    };
    resolve(line, &ctx)
}

async fn speech(core: &Core, voice: &str, text: &str, rate: f64, prio: SpeakPriority) -> Result<Arc<[i16]>, String> {
    let key = (voice.to_owned(), (rate * 1000.0).round() as u32, text.to_owned());
    if let Some(p) = core.speech.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return Ok(p);
    }
    let opts = SpeakOpts {
        rate: rate as f32,
        ..SpeakOpts::default()
    };
    let s = core
        .deps
        .inference
        .speak(voice, text, opts, prio)
        .await
        .map_err(|e| e.to_string())?;
    let pcm: Arc<[i16]> = s.samples.into();
    if let Ok(mut m) = core.speech.lock() {
        m.insert(key, pcm.clone());
    }
    Ok(pcm)
}

/// A clip's prepared 48 kHz audio.
pub async fn clip_pcm(core: &Core, h: &BlobHash) -> Result<Arc<[i16]>, String> {
    if let Some(p) = core.clip_pcm.lock().ok().and_then(|m| m.get(h).cloned()) {
        return Ok(p);
    }
    let bytes = core
        .deps
        .blobs
        .get(h)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("clip {h} is missing"))?;
    let pcm = pb_audio::decode(&bytes, Some("wav")).map_err(|e| e.to_string())?;
    let at48 = pb_audio::resample(&pcm.samples, pcm.rate, pb_audio::PLAY_RATE).map_err(|e| e.to_string())?;
    let p: Arc<[i16]> = pb_audio::to_i16(&at48).into();
    if let Ok(mut m) = core.clip_pcm.lock() {
        m.insert(*h, p.clone());
    }
    Ok(p)
}

/// The fields every line can use, for a person in a channel, in `lang`.
pub fn fields_for(
    core: &Core,
    guild: GuildId,
    person: Option<UserId>,
    channel: Option<pb_domain::ChannelId>,
    lang: &Lang,
    label: Option<pb_domain::Label>,
    base: &Fields,
) -> Fields {
    let mut f = base.clone();
    let gs = core.guilds();
    if let Some(u) = person {
        // The name: the person's written name for this language, else how Fluxer shows them.
        let tree = core.settings.current();
        let written = tree
            .scoped_slots(guild, Some(u))
            .person
            .and_then(|s| s.get(&pb_voicelines::LineKey(Line::Name)))
            .and_then(|slot| slot.text.get(lang).or_else(|| slot.text.get(&lang.base())).cloned());
        f.insert(Field::Name, written.unwrap_or_else(|| gs.name(guild, u)));
    }
    if let Some(l) = label {
        f.insert(Field::Label, label_name(Locale::for_lang(lang), l));
    }
    // An action's length arrives in seconds and is spoken in this language.
    if let Some(secs) = f.get(&Field::Duration).and_then(|s| s.parse::<u64>().ok()) {
        f.insert(
            Field::Duration,
            pb_i18n::spoken_duration(Locale::for_lang(lang), std::time::Duration::from_secs(secs)),
        );
    }
    f.insert(Field::Server, gs.guild_name(guild));
    if let Some(c) = channel {
        f.insert(Field::Channel, gs.channel_name(guild, c));
    }
    f
}

/// Renders a line (or an exact text) for a person.
#[allow(clippy::too_many_arguments)]
pub async fn render(
    core: &Core,
    guild: GuildId,
    channel: Option<pb_domain::ChannelId>,
    person: Option<UserId>,
    line: &Line,
    exact: Option<&(Lang, String)>,
    heard: Option<ClfLang>,
    label: Option<pb_domain::Label>,
    base: &Fields,
    prio: SpeakPriority,
) -> Result<Rendered, String> {
    let tree = core.settings.current();
    let eff = tree.effective(Some(guild), person);
    let langs = languages(&tree, guild, person, heard);
    let res = match exact {
        Some((lang, text)) => Resolution::Text {
            source: pb_voicelines::Source::Person,
            key: pb_voicelines::LineKey(line.clone()),
            lang: lang.clone(),
            text: text.clone(),
        },
        None => resolve_line(core, guild, person, line, &langs),
    };
    let (lang, key) = match &res {
        Resolution::Text { lang, key, .. } => (Some(lang.clone()), Some(key.to_string())),
        Resolution::Clips { lang, key, .. } => (lang.clone(), Some(key.to_string())),
        Resolution::Silent => (None, None),
    };
    let fields = fields_for(
        core,
        guild,
        person,
        channel,
        lang.as_ref().unwrap_or(&langs[0]),
        label,
        base,
    );
    // A recorded name in the line's language is spliced in at {name}.
    let name_clip = match (&res, person, &lang) {
        (Resolution::Text { .. }, Some(u), Some(l)) => {
            match resolve_line(core, guild, Some(u), &Line::Name, std::slice::from_ref(l)) {
                Resolution::Clips {
                    source: pb_voicelines::Source::Person,
                    clips,
                    ..
                } => clips.first().copied(),
                _ => None,
            }
        }
        _ => None,
    };
    let memory: pb_voicelines::SaidTo = (guild, person, key.clone().unwrap_or_default());
    let last = core.no_repeat.lock().ok().and_then(|n| n.last(&memory).copied());
    let p = plan(&res, &fields, name_clip.as_ref(), last.as_ref(), &mut |n| {
        fastrand::usize(..n.max(1))
    });
    let voices = core.deps.inference.voices();
    let rate = eff.speech_rate.value.get();
    let mut out = Rendered {
        lang,
        line: key,
        ..Rendered::default()
    };
    let mut said: Vec<String> = Vec::new();
    for part in &p.parts {
        match part {
            Part::Speak { lang, text } => {
                let voice =
                    voice_for(&voices, &eff.tts_voices.value, lang).ok_or_else(|| format!("no voice speaks {lang}"))?;
                let pcm = speech(core, &voice, text, rate, prio).await?;
                out.pcm.extend_from_slice(&pcm);
                said.push(text.clone());
            }
            Part::Clip(h) => {
                let pcm = clip_pcm(core, h).await?;
                out.pcm.extend_from_slice(&pcm);
                out.clips.push(*h);
                if let Some(t) = core.clip(h).and_then(|c| c.transcript).or_else(|| {
                    core.deps
                        .shipped_clips
                        .iter()
                        .find(|s| s.hash == *h)
                        .map(|s| s.text.clone())
                }) {
                    said.push(t);
                }
                if let Ok(mut n) = core.no_repeat.lock() {
                    n.remember(memory.clone(), *h);
                }
            }
            Part::Silence { ms } => out.pcm.extend(std::iter::repeat_n(
                0i16,
                (*ms as usize) * pb_audio::PLAY_RATE as usize / 1000,
            )),
        }
    }
    if !said.is_empty() {
        out.text = Some(said.join(" "));
    }
    Ok(out)
}

/// Renders the warnings a person is likely to get (every enabled type × step), so playing never waits for speech.
pub async fn prerender(core: Arc<Core>, guild: GuildId, user: UserId) {
    let tree = core.settings.current();
    let eff = tree.effective(Some(guild), Some(user));
    let steps = u32::try_from(eff.escalation.value.steps().len()).unwrap_or(1);
    for label in eff.enabled_labels() {
        for step in 1..=steps {
            let line = Line::Warning {
                label: pb_voicelines::Sel::Is(label),
                step: pb_voicelines::Sel::Is(step),
            };
            let mut f = Fields::new();
            f.insert(Field::Step, step.to_string());
            f.insert(Field::Count, step.to_string());
            f.insert(Field::Strikes, eff.strikes.value.get().to_string());
            // Only text lines need rendering; clips are decoded on first use.
            if let Err(e) = render(
                &core,
                guild,
                None,
                Some(user),
                &line,
                None,
                None,
                Some(label),
                &f,
                SpeakPriority::Prerender,
            )
            .await
            {
                tracing::debug!(error = %e, "a warning could not be rendered ahead of time");
            }
        }
    }
}
