//! Text → Piper phonemes and ids, ported from piper-tts 1.8.0 (`voice.py` `PiperVoice.phonemize` for espeak voices,
//! `phonemize_espeak.py`, `phoneme_ids.py`) on the same espeak-ng (pinned commit, see `pb-espeak`):
//!
//! - `[[ … ]]` blocks in the text are raw phonemes, taken character by character;
//! - other text goes to espeak-ng clause by clause: language-switch flags `(xx)` removed, the clause's punctuation kept
//!   (`, : ;` followed by a space), every character decomposed (NFD) into its own phoneme, sentences end where espeak
//!   says a clause ends a sentence, and the voice's known vowel clusters are merged back into one phoneme;
//! - ids are `^ _ (id _)* $`; phonemes missing from the voice's map are skipped (and reported).

use std::collections::{BTreeSet, HashMap};

use pb_espeak::{Espeak, EspeakError, Terminator};
use unicode_normalization::UnicodeNormalization;

/// Phonemes of one sentence (each entry one phoneme, usually one character).
pub type Sentence = Vec<String>;

/// Piper's `_PHONEME_BLOCK_PATTERN.split(text)` with `(\[\[.*?\]\])`: text and `[[…]]` blocks, in order. A block
/// ends at the first `]]`; `.` does not match a newline, so a block cannot contain one.
fn split_blocks(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut last = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find("[[") {
        let start = from + found;
        let body = &text[start + 2..];
        match body.find("]]") {
            Some(end) if !body[..end].contains('\n') => {
                let stop = start + 2 + end + 2;
                parts.push(&text[last..start]);
                parts.push(&text[start..stop]);
                last = stop;
                from = stop;
            }
            _ => from = start + 1,
        }
    }
    parts.push(&text[last..]);
    parts
}

/// `re.sub(r"\([^)]+\)", "", s)`: removes espeak's language-switch flags such as `(en)`. At each `(` the regex takes
/// everything up to the first `)` if there is at least one character in between.
fn strip_language_flags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(inner) = rest.strip_prefix('(')
            && let Some(close) = inner.find(')')
            && close > 0
        {
            i += 1 + close + 1;
            continue;
        }
        let c = rest.chars().next().expect("non-empty");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `_merge_known_vowel_clusters`.
fn merge_vowel_clusters(phones: Vec<String>, clusters: &BTreeSet<Vec<String>>) -> Vec<String> {
    let Some(max_len) = clusters.iter().map(Vec::len).max() else {
        return phones;
    };
    let mut out = Vec::with_capacity(phones.len());
    let mut i = 0;
    while i < phones.len() {
        let mut matched = None;
        for n in (2..=max_len.min(phones.len() - i)).rev() {
            if clusters.contains(&phones[i..i + n].to_vec()) {
                matched = Some(n);
                break;
            }
        }
        match matched {
            Some(n) => {
                out.push(phones[i..i + n].concat());
                i += n;
            }
            None => {
                out.push(phones[i].clone());
                i += 1;
            }
        }
    }
    out
}

/// `EspeakPhonemizer.phonemize`: espeak text → sentences of phonemes.
fn espeak_sentences(
    espeak: &mut Espeak,
    voice: &str,
    text: &str,
    clusters: Option<&BTreeSet<Vec<String>>>,
) -> Result<Vec<Sentence>, EspeakError> {
    let mut all = Vec::new();
    let mut sentence: Sentence = Vec::new();
    let finish = |s: Sentence| match clusters {
        Some(c) if !c.is_empty() => merge_vowel_clusters(s, c),
        _ => s,
    };
    for clause in espeak.clauses(voice, text)? {
        let mut s = strip_language_flags(&clause.phonemes);
        s.push_str(clause.terminator.as_str());
        if matches!(
            clause.terminator,
            Terminator::Comma | Terminator::Colon | Terminator::Semicolon
        ) {
            s.push(' ');
        }
        sentence.extend(s.nfd().map(String::from));
        if clause.end_of_sentence {
            all.push(finish(std::mem::take(&mut sentence)));
        }
    }
    if !sentence.is_empty() {
        all.push(finish(sentence));
    }
    Ok(all)
}

/// `PiperVoice.phonemize` for espeak voices.
pub fn phonemize(
    espeak: &mut Espeak,
    voice: &str,
    text: &str,
    clusters: Option<&BTreeSet<Vec<String>>>,
) -> Result<Vec<Sentence>, EspeakError> {
    let parts = split_blocks(text);
    let mut phonemes: Vec<Sentence> = Vec::new();
    let mut prev_raw = false;
    for (i, part) in parts.iter().enumerate() {
        if part.starts_with("[[") {
            prev_raw = true;
            if phonemes.is_empty() {
                phonemes.push(Vec::new());
            }
            let last = phonemes.last_mut().expect("just ensured");
            if i > 0 && parts[i - 1].ends_with(' ') {
                last.push(" ".into());
            }
            last.extend(part[2..part.len() - 2].trim().chars().map(String::from));
            if i + 1 < parts.len() && parts[i + 1].starts_with(' ') {
                last.push(" ".into());
            }
            continue;
        }
        let mut found = espeak_sentences(espeak, voice, part, clusters)?;
        if prev_raw && !found.is_empty() {
            let first = found.remove(0);
            if let Some(last) = phonemes.last_mut() {
                last.extend(first);
            }
        }
        phonemes.extend(found);
        prev_raw = false;
    }
    if phonemes.last().is_some_and(Vec::is_empty) {
        phonemes.pop();
    }
    Ok(phonemes)
}

/// `phonemes_to_ids`: `^ _ (id _)* $`. Phonemes missing from the map are skipped; they are returned so the caller can
/// report them.
pub fn to_ids(phonemes: &[String], id_map: &HashMap<String, Vec<i64>>) -> (Vec<i64>, Vec<String>) {
    let get = |p: &str| id_map.get(p).cloned().unwrap_or_default();
    let pad = get("_");
    let mut ids = get("^");
    ids.extend(&pad);
    let mut missing = Vec::new();
    for p in phonemes {
        match id_map.get(p) {
            Some(found) => {
                ids.extend(found);
                ids.extend(&pad);
            }
            None => missing.push(p.clone()),
        }
    }
    ids.extend(get("$"));
    (ids, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_raw_phoneme_blocks_like_piper() {
        assert_eq!(split_blocks("plain"), vec!["plain"]);
        assert_eq!(split_blocks("Hi [[hˈaɪ]] there"), vec!["Hi ", "[[hˈaɪ]]", " there"]);
        assert_eq!(split_blocks("[[a]][[b]]"), vec!["", "[[a]]", "", "[[b]]", ""]);
        assert_eq!(split_blocks("x [[a\nb]] y"), vec!["x [[a\nb]] y"]);
    }

    #[test]
    fn strips_language_flags_like_the_regex() {
        assert_eq!(strip_language_flags("(en)hˈɛloʊ(de) vˈɛlt"), "hˈɛloʊ vˈɛlt");
        assert_eq!(strip_language_flags("a () b"), "a () b");
    }

    #[test]
    fn merges_vowel_clusters() {
        let clusters: BTreeSet<Vec<String>> = [vec!["a".into(), "ɪ".into()]].into_iter().collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(merge_vowel_clusters(s(&["h", "a", "ɪ"]), &clusters), s(&["h", "aɪ"]));
    }
}
