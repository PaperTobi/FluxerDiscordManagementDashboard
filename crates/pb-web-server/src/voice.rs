//! Voice lines and the clip library: editing slots, uploading (any audio format, streamed to disk), renaming and
//! removing clips.

use axum::Form;
use axum::extract::{Multipart, State};
use axum::response::Response;
use http::HeaderMap;
use pb_domain::{BlobHash, Label, Lang};
use pb_i18n::text;
use pb_voicelines::{Line, LineKey, Sel, Slot};

use tokio::io::AsyncWriteExt;

use super::forms::{Fields, Sender, ask_first, confirmed, field, parse_scope};
use super::server::WebState;
use super::util::{locale_of, same_origin};
use pb_web::fmt::engine_error;

/// The line a form is about: `line`, or a new one from `line_kind`/`line_label`/`line_step`/`line_preset`.
fn line_of(f: &Fields) -> Option<LineKey> {
    if let Some(l) = field(f, "line") {
        return l.parse().ok();
    }
    match field(f, "line_kind")? {
        "warning" => {
            let label = match field(f, "line_label").unwrap_or("any") {
                "any" => Sel::Any,
                l => Sel::Is(l.parse::<Label>().ok()?),
            };
            let step = match field(f, "line_step").map(str::trim).filter(|s| !s.is_empty()) {
                None => Sel::Any,
                Some(n) => Sel::Is(n.parse::<u32>().ok().filter(|n| *n > 0)?),
            };
            Some(LineKey(Line::Warning { label, step }))
        }
        "say" => {
            let preset = field(f, "line_preset")?.trim().to_lowercase().replace(' ', "-");
            format!("say.{preset}").parse().ok()
        }
        _ => None,
    }
}

/// `POST /voice-lines`: `op` = `set_text` (lang, text), `remove_text` (lang), `add_clip` (clip), `remove_clip`
/// (clip), `clear`.
pub async fn edit(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let loc = s.locale;
    let (Some(scope), Some(key)) = (field(&f, "scope").and_then(parse_scope), line_of(&f)) else {
        return st.done(&s, &f, false, text(loc, "ui-not-a-line", &[]));
    };
    if !s.may_change(scope) {
        return st.done(&s, &f, false, text(loc, "ui-not-allowed", &[]));
    }
    if matches!(key.0, Line::Name) && !matches!(scope, pb_domain::Scope::Person { .. }) {
        return st.done(&s, &f, false, text(loc, "ui-not-a-line", &[]));
    }
    let lang = field(&f, "lang").and_then(|l| l.parse::<Lang>().ok());
    let clip = field(&f, "clip").and_then(|c| c.parse::<BlobHash>().ok());
    let op = field(&f, "op").unwrap_or_default().to_owned();
    if op == "add_clip" && clip.is_none_or(|c| st.engine.clip(&c).is_none()) {
        return st.done(&s, &f, false, text(loc, "ui-no-such-clip", &[]));
    }
    let said = field(&f, "text").map(|t| t.trim().to_owned()).unwrap_or_default();
    let result = st
        .engine
        .settings()
        .change(s.actor(), move |t| {
            let mut slot: Slot = t
                .voice_lines(scope)
                .and_then(|x| x.get(&key))
                .cloned()
                .unwrap_or_default();
            match (op.as_str(), lang, clip) {
                ("set_text", Some(l), _) if !said.is_empty() => {
                    slot.text.insert(l, said);
                }
                ("remove_text", Some(l), _) => {
                    slot.text.remove(&l);
                }
                ("add_clip", _, Some(c)) => {
                    if !slot.clips.contains(&c) {
                        slot.clips.push(c);
                    }
                }
                ("remove_clip", _, Some(c)) => slot.clips.retain(|x| *x != c),
                ("clear", _, _) => slot = Slot::default(),
                _ => return Ok(Vec::new()),
            }
            Ok(t.set_voice_line(scope, key, Some(slot)).into_iter().collect())
        })
        .await;
    match result {
        Ok(c) if c.is_empty() => st.done(&s, &f, true, text(loc, "ui-unchanged", &[])),
        Ok(_) => st.done(&s, &f, true, text(loc, "ui-saved", &[])),
        Err(e) => st.done(&s, &f, false, super::forms::change_error(loc, &e)),
    }
}

/// `POST /clips` (multipart: `csrf` first, then `file`, `name`, `lang`, `back`): adds a clip to the library. Nothing
/// is read from a request without a login and a same-origin header, and no file without the form token before it;
/// the file goes to disk as it arrives (any size).
pub async fn upload(State(st): State<WebState>, headers: HeaderMap, mut mp: Multipart) -> Response {
    let loc = locale_of(&headers);
    let refused = |st: &WebState| st.done_anon(&headers, false, text(loc, "form-expired", &[]));
    let Some(login) = st.sessions.lookup(&headers) else {
        return refused(&st);
    };
    if !same_origin(&headers) {
        return refused(&st);
    }
    let mut fields: Fields = Vec::new();
    let mut file: Option<(std::path::PathBuf, Option<String>, String)> = None;
    let failed = async |file: Option<(std::path::PathBuf, Option<String>, String)>| {
        if let Some((path, _, _)) = file {
            let _ = tokio::fs::remove_file(path).await;
        }
    };
    loop {
        let mut part = match mp.next_field().await {
            Ok(Some(part)) => part,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "an upload could not be read");
                failed(file).await;
                return st.done_anon(&headers, false, text(loc, "ui-upload-failed", &[]));
            }
        };
        let name = part.name().unwrap_or_default().to_owned();
        if name != "file" {
            if let Ok(v) = part.text().await {
                fields.push((name, v));
            }
            continue;
        }
        let token_ok = field(&fields, "csrf").is_some_and(|t| st.sessions.csrf_ok(&login.key, t));
        if !token_ok || file.is_some() {
            failed(file).await;
            return refused(&st);
        }
        let file_name = part.file_name().unwrap_or("clip").to_owned();
        let ext = std::path::Path::new(&file_name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        let path = st
            .engine
            .upload_dir()
            .join(format!("upload-{}.part", super::auth::random_token(12)));
        file = Some((path.clone(), ext, file_name));
        let written = async {
            let mut out = tokio::fs::File::create(&path).await?;
            while let Some(chunk) = part.chunk().await.map_err(std::io::Error::other)? {
                out.write_all(&chunk).await?;
            }
            out.flush().await
        };
        if let Err(e) = written.await {
            tracing::warn!(error = %e, "an upload broke off");
            failed(file).await;
            return st.done_anon(&headers, false, text(loc, "ui-upload-failed", &[]));
        }
    }
    let s = match st.sender(&headers, &fields) {
        Ok(s) => s,
        Err(r) => {
            failed(file).await;
            return *r;
        }
    };
    let Some((path, ext, file_name)) = file else {
        return st.done(&s, &fields, false, text(loc, "ui-upload-empty", &[]));
    };
    if tokio::fs::metadata(&path).await.is_ok_and(|m| m.len() == 0) {
        let _ = tokio::fs::remove_file(&path).await;
        return st.done(&s, &fields, false, text(loc, "ui-upload-empty", &[]));
    }
    let name = field(&fields, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            std::path::Path::new(&file_name)
                .file_stem()
                .map_or_else(|| "clip".to_owned(), |s| s.to_string_lossy().into_owned())
        });
    let lang = field(&fields, "lang").and_then(|l| l.parse::<Lang>().ok());
    match st.engine.add_clip(path, name.clone(), ext, lang, s.actor()).await {
        Ok(_) => st.done(&s, &fields, true, text(loc, "ui-clip-added", &[("name", name.into())])),
        Err(e) => st.done(&s, &fields, false, engine_error(loc, &e)),
    }
}

/// `POST /clips/update`: name, language, transcript.
pub async fn update(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(clip) = field(&f, "clip").and_then(|c| c.parse::<BlobHash>().ok()) else {
        return st.done(&s, &f, false, text(s.locale, "ui-no-such-clip", &[]));
    };
    if !st.may_edit_clip(&s, &clip) {
        return st.done(&s, &f, false, text(s.locale, "ui-clip-not-yours", &[]));
    }
    let name = field(&f, "name").map(str::trim).unwrap_or_default().to_owned();
    if name.is_empty() {
        return st.done(&s, &f, false, text(s.locale, "ui-clip-name-empty", &[]));
    }
    let lang = field(&f, "lang").and_then(|l| l.parse::<Lang>().ok());
    let transcript = field(&f, "transcript").map(str::to_owned);
    match st.engine.update_clip(clip, name, lang, transcript, s.actor()).await {
        Ok(_) => st.done(&s, &f, true, text(s.locale, "ui-saved", &[])),
        Err(e) => st.done(&s, &f, false, engine_error(s.locale, &e)),
    }
}

/// `POST /clips/remove`: asks first when a voice line uses the clip.
pub async fn remove(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(clip) = field(&f, "clip").and_then(|c| c.parse::<BlobHash>().ok()) else {
        return st.done(&s, &f, false, text(s.locale, "ui-no-such-clip", &[]));
    };
    if !st.may_edit_clip(&s, &clip) {
        return st.done(&s, &f, false, text(s.locale, "ui-clip-not-yours", &[]));
    }
    let used = !pb_web::pages::voicelines::clip_uses(&st.engine.settings().current(), &clip).is_empty();
    if used && !confirmed(&f) {
        return ask_first("clip", &f, &["clip"]);
    }
    match st.engine.remove_clip(clip, s.actor()).await {
        Ok(()) => st.done(&s, &f, true, text(s.locale, "ui-clip-removed-notice", &[])),
        Err(e) => st.done(&s, &f, false, engine_error(s.locale, &e)),
    }
}

impl WebState {
    /// The clip's uploader and the owner may change or remove it (unknown clips are left to the engine's answer).
    fn may_edit_clip(&self, s: &Sender, clip: &BlobHash) -> bool {
        self.engine
            .clip(clip)
            .is_none_or(|c| c.editable_by(s.login.record.user, s.access.owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(pairs: &[(&str, &str)]) -> Fields {
        pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn lines_from_forms() {
        assert_eq!(
            line_of(&f(&[("line", "greeting")])).map(|k| k.to_string()).as_deref(),
            Some("greeting")
        );
        let k = line_of(&f(&[
            ("line_kind", "warning"),
            ("line_label", "profanity"),
            ("line_step", "2"),
        ]));
        assert_eq!(k.map(|k| k.to_string()).as_deref(), Some("warning.profanity.2"));
        let k = line_of(&f(&[
            ("line_kind", "warning"),
            ("line_label", "any"),
            ("line_step", ""),
        ]));
        assert_eq!(k.map(|k| k.to_string()).as_deref(), Some("warning.any.any"));
        let k = line_of(&f(&[("line_kind", "say"), ("line_preset", "Calm Down")]));
        assert_eq!(k.map(|k| k.to_string()).as_deref(), Some("say.calm-down"));
        assert!(line_of(&f(&[("line_kind", "warning"), ("line_step", "0")])).is_none());
    }
}
