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

use super::core::{Core, lock};
use super::error::RenderError;

/// What is said exactly as given instead of resolving a voice line ("Say now", a preview in one language).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exact {
    /// A text, spoken in this language (placeholders filled in).
    Text(Lang, String),
    /// A clip, played as it is.
    Clip(BlobHash),
}

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

/// The installed voice (as `<model>:<id>`) for a line in a language: the voice chosen for its kind of line when it
/// speaks the language, else the one chosen for the language, else the first installed one for that language (medium
/// quality first: the old bot's default `en_US-lessac-medium`).
pub fn voice_for(
    voices: &[VoiceInfo],
    line_voice: Option<&str>,
    chosen: &std::collections::BTreeMap<Lang, String>,
    lang: &Lang,
) -> Option<String> {
    let want = lang.to_string().replace('-', "_").to_ascii_lowercase();
    let speaks = |v: &VoiceInfo| {
        std::iter::once(&v.language).chain(&v.languages).any(|l| {
            let code = l.replace('-', "_").to_ascii_lowercase();
            code == want || code.split('_').next() == Some(lang.language())
        })
    };
    let installed = |id: &str| voices.iter().find(|v| v.named(id));
    if let Some(v) = line_voice.and_then(installed).filter(|v| speaks(v)) {
        return Some(v.full_id());
    }
    if let Some(v) = chosen
        .get(lang)
        .or_else(|| chosen.get(&lang.base()))
        .and_then(|id| installed(id))
    {
        return Some(v.full_id());
    }
    let mut fitting: Vec<&VoiceInfo> = voices.iter().filter(|v| speaks(v)).collect();
    fitting.sort_by_key(|v| (v.quality != "medium", v.quality != "high", v.id.clone()));
    fitting.first().map(|v| v.full_id())
}

/// The voice chosen for a line's kind (none for a name: it is said in the voice of the line it is part of).
fn line_voice<'a>(eff: &'a pb_settings::Effective, line: &Line) -> Option<&'a str> {
    line.kind()
        .and_then(|k| eff.line_voices.value.get(&k))
        .map(String::as_str)
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
    let has_voice = |l: &Lang| voice_for(&voices, line_voice(&eff, line), &eff.tts_voices.value, l).is_some();
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

/// What a voice line says, written (for the chat): in the person's first language that has a text for it (no voice
/// needed), or the transcripts of its clips. `None` when it has neither.
pub fn line_text(
    core: &Core,
    guild: GuildId,
    person: UserId,
    channel: Option<pb_domain::ChannelId>,
    line: &Line,
    label: Option<pb_domain::Label>,
    base: &Fields,
) -> Option<(Lang, String)> {
    let tree = core.settings.current();
    let langs = languages(&tree, guild, Some(person), None);
    let shipped: Vec<BlobHash> = core.deps.shipped_clips.iter().map(|s| s.hash).collect();
    let info = clip_info(core);
    let ctx = ResolveCtx {
        languages: &langs,
        slots: tree.scoped_slots(guild, Some(person)),
        clip: &info,
        has_voice: &|_| true,
        fallback_clips: &shipped,
    };
    let res = resolve(line, &ctx);
    let lang = match &res {
        Resolution::Text { lang, .. } => lang.clone(),
        Resolution::Clips { lang, .. } => lang.clone().unwrap_or_else(|| langs[0].clone()),
        Resolution::Silent => return None,
    };
    let fields = fields_for(core, guild, Some(person), channel, &lang, label, base);
    let p = plan(&res, &fields, None, None, &mut |_| 0);
    let said: Vec<String> = p
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Speak { text, .. } => Some(text.clone()),
            Part::Clip(h) => core.clip(h).and_then(|c| c.transcript).or_else(|| {
                core.deps
                    .shipped_clips
                    .iter()
                    .find(|s| s.hash == *h)
                    .map(|s| s.text.clone())
            }),
            Part::Silence { .. } => None,
        })
        .collect();
    (!said.is_empty()).then(|| (lang, said.join(" ")))
}

/// A phrase's audio: from the cache, from a render already running (unless that one runs at a lower priority than
/// this caller needs), or rendered now.
async fn speech(
    core: &Core,
    voice: &str,
    text: &str,
    rate: f64,
    prio: SpeakPriority,
) -> Result<Arc<[i16]>, RenderError> {
    let key = (voice.to_owned(), (rate * 1000.0).round() as u32, text.to_owned());
    if let Some(p) = lock(&core.speech).get(&key) {
        return Ok(p);
    }
    let cell = {
        let mut inflight = lock(&core.speech_inflight);
        match inflight.get(&key) {
            Some((running, cell)) if *running >= prio => cell.clone(),
            _ => {
                let cell = Arc::new(tokio::sync::OnceCell::new());
                inflight.insert(key.clone(), (prio, cell.clone()));
                cell
            }
        }
    };
    let result = cell
        .get_or_init(|| async {
            let opts = SpeakOpts {
                rate: rate as f32,
                ..SpeakOpts::default()
            };
            let s = core.deps.inference.speak(voice, text, opts, prio).await?;
            let pcm: Arc<[i16]> = s.samples.into();
            lock(&core.speech).insert(key.clone(), pcm.clone());
            Ok(pcm)
        })
        .await
        .clone();
    let mut inflight = lock(&core.speech_inflight);
    if inflight.get(&key).is_some_and(|(_, c)| Arc::ptr_eq(c, &cell)) {
        inflight.remove(&key);
    }
    result
}

/// A clip's prepared 48 kHz audio.
pub async fn clip_pcm(core: &Core, h: &BlobHash) -> Result<Arc<[i16]>, RenderError> {
    if let Some(p) = lock(&core.clip_pcm).get(h) {
        return Ok(p);
    }
    let bytes = core.deps.blobs.get(h).await?.ok_or(RenderError::ClipMissing(*h))?;
    let unreadable = |error| RenderError::ClipUnreadable { clip: *h, error };
    let pcm = pb_audio::decode(&bytes, Some("wav")).map_err(unreadable)?;
    let at48 = pb_audio::resample(&pcm.samples, pcm.rate, pb_audio::PLAY_RATE).map_err(unreadable)?;
    let p: Arc<[i16]> = pb_audio::to_i16(&at48).into();
    lock(&core.clip_pcm).insert(*h, p.clone());
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

/// Renders a line (or an exact text) for a person. Only what is played is remembered for "not the same clip twice in
/// a row" (`remember`); previews and renders ahead of time leave that memory alone.
#[allow(clippy::too_many_arguments)]
pub async fn render(
    core: &Core,
    guild: GuildId,
    channel: Option<pb_domain::ChannelId>,
    person: Option<UserId>,
    line: &Line,
    exact: Option<&Exact>,
    heard: Option<ClfLang>,
    label: Option<pb_domain::Label>,
    base: &Fields,
    prio: SpeakPriority,
    remember: bool,
) -> Result<Rendered, RenderError> {
    let tree = core.settings.current();
    let eff = tree.effective(Some(guild), person);
    let langs = languages(&tree, guild, person, heard);
    let res = match exact {
        Some(Exact::Text(lang, text)) => Resolution::Text {
            source: pb_voicelines::Source::Person,
            key: pb_voicelines::LineKey(line.clone()),
            lang: lang.clone(),
            text: text.clone(),
        },
        Some(Exact::Clip(h)) => Resolution::Clips {
            source: pb_voicelines::Source::Person,
            key: pb_voicelines::LineKey(line.clone()),
            lang: core.clip(h).and_then(|c| c.lang),
            clips: vec![*h],
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
    let last = lock(&core.no_repeat).last(&memory).copied();
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
                let voice = voice_for(&voices, line_voice(&eff, line), &eff.tts_voices.value, lang)
                    .ok_or_else(|| RenderError::NoVoice(lang.clone()))?;
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
                if remember {
                    lock(&core.no_repeat).remember(memory.clone(), *h);
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

/// What a person's next warning would be: its count (violations in the window plus this one), the call they are in
/// and the language last heard from them. Rendered with exactly the fields a live warning gets, so it is found in the
/// cache when it is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextWarning {
    pub guild: GuildId,
    pub user: UserId,
    pub channel: Option<pb_domain::ChannelId>,
    pub count: u32,
    pub heard: Option<ClfLang>,
}

/// The fields of a warning (shared by moderation and the renders ahead of time, so both give the same text).
pub fn warning_fields(eff: &pb_settings::Effective, step: u32, count: u32) -> Fields {
    let mut f = Fields::new();
    f.insert(Field::Step, step.to_string());
    f.insert(Field::Count, count.to_string());
    f.insert(Field::Strikes, eff.strikes.value.get().to_string());
    f
}

/// Renders the person's next warning for every enabled type, at the lowest priority, so playing never waits for
/// speech. Nothing is remembered as said.
pub async fn prerender(core: Arc<Core>, next: NextWarning) {
    let NextWarning {
        guild,
        user,
        channel,
        count,
        heard,
    } = next;
    let eff = core.settings.current().effective(Some(guild), Some(user));
    let step = eff.escalation.value.step_for(count).map_or(1, |(n, _)| n);
    let fields = warning_fields(&eff, step, count);
    for label in eff.enabled_labels() {
        let line = Line::Warning {
            label: pb_voicelines::Sel::Is(label),
            step: pb_voicelines::Sel::Is(step),
        };
        if let Err(e) = render(
            &core,
            guild,
            channel,
            Some(user),
            &line,
            None,
            heard,
            Some(label),
            &fields,
            SpeakPriority::Prerender,
            false,
        )
        .await
        {
            tracing::debug!(error = %e, "a warning could not be rendered ahead of time");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(model: &str, id: &str, languages: &[&str], quality: &str) -> VoiceInfo {
        VoiceInfo {
            id: id.into(),
            model: model.into(),
            language: languages[0].into(),
            languages: languages.iter().map(|&l| l.into()).collect(),
            speakers: vec![],
            sample_rate: 22_050,
            quality: quality.into(),
        }
    }

    #[test]
    fn the_line_voice_speaks_its_languages_and_the_language_voice_the_rest() {
        let voices = [
            voice("piper", "de_DE-thorsten-high", &["de_DE"], "high"),
            voice("piper", "en_US-amy-low", &["en_US"], "low"),
            voice("piper", "en_US-lessac-medium", &["en_US"], "medium"),
            voice("omni", "anna", &["en", "de", "fr"], "x"),
        ];
        let lang = |l: &str| l.parse::<Lang>().expect("lang");
        let mut chosen = std::collections::BTreeMap::new();
        // Nothing chosen: the language's first installed voice, medium quality first.
        assert_eq!(
            voice_for(&voices, None, &chosen, &lang("en")).as_deref(),
            Some("piper:en_US-lessac-medium")
        );
        // Chosen for the language, by bare id.
        chosen.insert(lang("en"), "en_US-amy-low".to_owned());
        assert_eq!(
            voice_for(&voices, None, &chosen, &lang("en-US")).as_deref(),
            Some("piper:en_US-amy-low")
        );
        // The line's voice wins where it speaks the language …
        let line = Some("omni:anna");
        assert_eq!(
            voice_for(&voices, line, &chosen, &lang("de")).as_deref(),
            Some("omni:anna")
        );
        assert_eq!(
            voice_for(&voices, line, &chosen, &lang("en")).as_deref(),
            Some("omni:anna")
        );
        // … and elsewhere the language's voice speaks.
        assert_eq!(
            voice_for(&voices, Some("piper:de_DE-thorsten-high"), &chosen, &lang("en")).as_deref(),
            Some("piper:en_US-amy-low")
        );
        // A voice that is not installed (any more) is skipped.
        assert_eq!(
            voice_for(&voices, Some("omni:gone"), &chosen, &lang("de")).as_deref(),
            Some("piper:de_DE-thorsten-high")
        );
        assert_eq!(voice_for(&voices, line, &chosen, &lang("ja")), None);
    }
}
