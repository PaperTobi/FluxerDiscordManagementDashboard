//! The four espeak-ng functions used (speak_lib.h). All unsafe code of this crate is here.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

const AUDIO_OUTPUT_SYNCHRONOUS: c_int = 2;
const ESPEAK_CHARS_AUTO: c_int = 0;
const ESPEAK_PHONEMES_IPA: c_int = 0x02;
const EE_OK: c_int = 0;

unsafe extern "C" {
    fn espeak_Initialize(output: c_int, buflength: c_int, path: *const c_char, options: c_int) -> c_int;
    fn espeak_SetVoiceByName(name: *const c_char) -> c_int;
    fn espeak_TextToPhonemesWithTerminator(
        textptr: *mut *const c_void,
        textmode: c_int,
        phonememode: c_int,
        terminator: *mut c_int,
    ) -> *const c_char;
    fn espeak_Terminate() -> c_int;
}

pub fn initialize(path: &CString) -> bool {
    // SAFETY: path is a valid NUL-terminated string; espeak-ng copies it.
    unsafe { espeak_Initialize(AUDIO_OUTPUT_SYNCHRONOUS, 0, path.as_ptr(), 0) >= 0 }
}

pub fn set_voice(name: &CString) -> bool {
    // SAFETY: name is a valid NUL-terminated string.
    unsafe { espeak_SetVoiceByName(name.as_ptr()) == EE_OK }
}

/// Phonemes and terminator flags for each clause, consuming the whole text (as Piper's bridge does).
pub fn phonemes(text: &CString) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    let mut ptr: *const c_void = text.as_ptr().cast();
    while !ptr.is_null() {
        let mut terminator: c_int = 0;
        // SAFETY: ptr points into `text` (alive for this call) or is advanced by espeak-ng within it; espeak-ng sets
        // it to NULL at the end. The returned string is espeak-ng's static buffer, copied before the next call.
        let phonemes = unsafe {
            let p =
                espeak_TextToPhonemesWithTerminator(&mut ptr, ESPEAK_CHARS_AUTO, ESPEAK_PHONEMES_IPA, &mut terminator);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        out.push((phonemes, terminator));
    }
    out
}

pub fn terminate() {
    // SAFETY: called once when the only Espeak is dropped.
    unsafe {
        espeak_Terminate();
    }
}
