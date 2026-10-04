//! How close the pure-Rust espeak-ng port is to the C library Piper's voices were trained with (docs/proposals/0003):
//! `cargo run --release -p pb-tts-piper --example port_parity`. When every voice reaches 100 %, the C exception in
//! docs/exceptions.toml can go.

use std::collections::HashMap;
use std::path::PathBuf;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

#[derive(serde::Deserialize)]
struct Golden {
    rows: Vec<Row>,
}
#[derive(serde::Deserialize)]
struct Row {
    text: String,
    ids: Vec<Vec<i64>>,
}

/// The same Piper-style pipeline on the pure-Rust espeak-ng port (its clause lines from `text_to_ipa`, terminators
/// from `translate_to_codes`).
struct Port {
    dir: PathBuf,
    translators: HashMap<String, espeak_ng::Translator>,
}

impl Port {
    fn new() -> Self {
        let dir = std::env::temp_dir().join("pb-espeak-port-data");
        std::fs::create_dir_all(&dir).expect("data dir");
        espeak_ng::install_bundled_data(&dir).expect("bundled data");
        Port {
            dir,
            translators: HashMap::new(),
        }
    }

    fn phonemize(&mut self, voice: &str, text: &str) -> Vec<Vec<String>> {
        use unicode_normalization::UnicodeNormalization;
        let dir = self.dir.clone();
        let t = self
            .translators
            .entry(voice.into())
            .or_insert_with(|| espeak_ng::Translator::new(voice, Some(&dir)).expect("voice"));
        let ipa = t.text_to_ipa(text).expect("ipa");
        let terms: Vec<char> = t
            .translate_to_codes(text)
            .expect("codes")
            .iter()
            .filter_map(|c| c.clause_char)
            .collect();
        let (mut out, mut sentence) = (Vec::new(), Vec::new());
        for (i, clause) in ipa.split('\n').enumerate() {
            let term = terms.get(i).copied().filter(|c| ".?!,:;".contains(*c));
            let mut s = clause.trim().to_owned();
            if let Some(c) = term {
                s.push(c);
                if ",:;".contains(c) {
                    s.push(' ');
                }
            }
            sentence.extend(s.nfd().map(String::from));
            if term.is_some_and(|c| ".?!".contains(c)) {
                out.push(std::mem::take(&mut sentence));
            }
        }
        if !sentence.is_empty() {
            out.push(sentence);
        }
        out
    }
}

fn main() {
    let mut port = Port::new();
    for (voice, espeak) in [("de_DE-thorsten-high", "de"), ("en_US-lessac-high", "en-us")] {
        let path = pb_testkit::weights()
            .join("voices")
            .join(voice)
            .join(format!("{voice}.onnx.json"));
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("config")).expect("json");
        let map: HashMap<String, Vec<i64>> = serde_json::from_value(config["phoneme_id_map"].clone()).expect("map");
        let g: Golden =
            serde_json::from_str(&std::fs::read_to_string(golden(&format!("phonemes_{voice}.json"))).expect("golden"))
                .expect("json");
        let same = g
            .rows
            .iter()
            .filter(|row| {
                let ids: Vec<Vec<i64>> = port
                    .phonemize(espeak, &row.text)
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| pb_tts_piper::phonemes::to_ids(s, &map).0)
                    .collect();
                ids == row.ids
            })
            .count();
        println!(
            "{voice}: {same}/{} sentences identical to Piper ({:.0} %)",
            g.rows.len(),
            100.0 * same as f64 / g.rows.len() as f64
        );
    }
}
