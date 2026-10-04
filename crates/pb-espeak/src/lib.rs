//! Phonemes from espeak-ng, exactly as Piper computes them for its voices (`espeakbridge.c` of piper-tts 1.8.0):
//! `espeak_TextToPhonemesWithTerminator` clause by clause, in IPA, with each clause's terminator.
//!
//! espeak-ng is a C library with global state, so the process has one instance: every [`Espeak`] handle shares it
//! (opening again with the same data reuses it; it ends when the last handle is dropped), and calls are serialised.
//! That lets a text-to-speech engine be rebuilt (new thread count, new voices) while the old one still exists.

mod ffi;

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EspeakError {
    #[error("espeak-ng is already open in this process with other data ({0})")]
    InUse(String),
    #[error("espeak-ng could not start with data in {0}")]
    Init(String),
    #[error("espeak-ng has no voice {0:?}")]
    Voice(String),
    #[error("text contains a NUL character")]
    Nul,
}

/// How a clause ended (Piper's mapping of espeak's clause flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    Period,
    Question,
    Exclamation,
    Comma,
    Colon,
    Semicolon,
    /// Anything else (no punctuation, other pause kinds).
    None,
}

impl Terminator {
    /// The punctuation Piper appends to the clause's phonemes.
    pub fn as_str(self) -> &'static str {
        match self {
            Terminator::Period => ".",
            Terminator::Question => "?",
            Terminator::Exclamation => "!",
            Terminator::Comma => ",",
            Terminator::Colon => ":",
            Terminator::Semicolon => ";",
            Terminator::None => "",
        }
    }
}

/// One clause: its IPA phonemes, how it ended, and whether it ended a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub phonemes: String,
    pub terminator: Terminator,
    pub end_of_sentence: bool,
}

/// The process's espeak-ng: its data directory, how many handles use it, and the voice set last.
struct Instance {
    data: PathBuf,
    handles: usize,
    voice: Option<String>,
}

static INSTANCE: Mutex<Option<Instance>> = Mutex::new(None);

fn instance() -> MutexGuard<'static, Option<Instance>> {
    INSTANCE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A handle to the process's espeak-ng.
#[derive(Debug)]
pub struct Espeak {
    _private: (),
}

/// Where `cargo xtask espeak-ng` put espeak-ng's data (the directory that contains `espeak-ng-data`).
pub const BUILD_DATA_DIR: &str = env!("PB_ESPEAK_NG_BUILD_DATA");

impl Espeak {
    /// A handle to espeak-ng with the data in `data_parent/espeak-ng-data` (started on first use).
    pub fn open(data_parent: &Path) -> Result<Self, EspeakError> {
        let mut inst = instance();
        match inst.as_mut() {
            Some(i) if i.data == data_parent => {
                i.handles += 1;
                return Ok(Espeak { _private: () });
            }
            Some(i) => return Err(EspeakError::InUse(i.data.display().to_string())),
            None => {}
        }
        let shown = data_parent.display().to_string();
        let path = CString::new(data_parent.as_os_str().as_encoded_bytes()).map_err(|_| EspeakError::Nul)?;
        if !ffi::initialize(&path) {
            return Err(EspeakError::Init(shown));
        }
        *inst = Some(Instance {
            data: data_parent.to_owned(),
            handles: 1,
            voice: None,
        });
        Ok(Espeak { _private: () })
    }

    /// Clauses of `text` spoken by espeak voice `voice` (e.g. "de", "en-us").
    pub fn clauses(&mut self, voice: &str, text: &str) -> Result<Vec<Clause>, EspeakError> {
        let text = CString::new(text).map_err(|_| EspeakError::Nul)?;
        // One call at a time: espeak-ng's state (the voice, its buffers) is global.
        let mut inst = instance();
        let Some(i) = inst.as_mut() else {
            return Err(EspeakError::Init("espeak-ng is not open".into()));
        };
        if i.voice.as_deref() != Some(voice) {
            let name = CString::new(voice).map_err(|_| EspeakError::Nul)?;
            if !ffi::set_voice(&name) {
                return Err(EspeakError::Voice(voice.to_owned()));
            }
            i.voice = Some(voice.to_owned());
        }
        let raw = ffi::phonemes(&text);
        drop(inst);
        Ok(raw
            .into_iter()
            .map(|(phonemes, flags)| {
                let flags = flags & 0x000F_FFFF;
                const SENTENCE: i32 = 0x0008_0000;
                const CLAUSE: i32 = 0x0004_0000;
                let terminator = match flags {
                    f if f == 40 | SENTENCE => Terminator::Period,
                    f if f == 40 | 0x2000 | SENTENCE => Terminator::Question,
                    f if f == 45 | 0x3000 | SENTENCE => Terminator::Exclamation,
                    f if f == 20 | 0x1000 | CLAUSE => Terminator::Comma,
                    f if f == 30 | CLAUSE => Terminator::Colon,
                    f if f == 30 | 0x1000 | CLAUSE => Terminator::Semicolon,
                    _ => Terminator::None,
                };
                Clause {
                    phonemes,
                    terminator,
                    end_of_sentence: flags & SENTENCE == SENTENCE,
                }
            })
            .collect())
    }
}

impl Drop for Espeak {
    fn drop(&mut self) {
        let mut inst = instance();
        let last = inst.as_mut().is_none_or(|i| {
            i.handles = i.handles.saturating_sub(1);
            i.handles == 0
        });
        if last && inst.take().is_some() {
            ffi::terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_share_one_instance() {
        let data = Path::new(BUILD_DATA_DIR);
        let mut a = Espeak::open(data).expect("first");
        let mut b = Espeak::open(data).expect("a second handle while the first lives");
        let x = a.clauses("en-us", "Hello there.").expect("phonemes");
        let y = b.clauses("de", "Hallo du.").expect("phonemes");
        assert!(!x.is_empty() && !y.is_empty());
        assert_eq!(
            a.clauses("en-us", "Hello there.").expect("again"),
            x,
            "the voice is set per call"
        );
        drop(a);
        assert!(b.clauses("de", "Hallo du.").is_ok(), "still open while a handle lives");
        drop(b);
        let mut c = Espeak::open(data).expect("opens again after the last handle went");
        assert!(c.clauses("en-us", "Hi.").is_ok());
        assert!(matches!(
            Espeak::open(Path::new("/elsewhere")),
            Err(EspeakError::InUse(_))
        ));
    }
}
