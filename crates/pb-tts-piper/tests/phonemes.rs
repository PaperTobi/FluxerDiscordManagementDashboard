//! Phonemes and ids against Piper 1.8.0 for the German and English corpora (recorded by `tools/golden/oracle.py`, a script now in the history at 18db450).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use pb_espeak::Espeak;
use pb_tts_piper::phonemes::{phonemize, to_ids};

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
    phonemes: Vec<String>,
    ids: Vec<Vec<i64>>,
}

fn voice_config(voice: &str) -> serde_json::Value {
    let path = pb_testkit::weights()
        .join("voices")
        .join(voice)
        .join(format!("{voice}.onnx.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("voice config")).expect("json")
}

fn compare(espeak: &mut Espeak, voice: &str) -> (usize, usize) {
    let config = voice_config(voice);
    let map: HashMap<String, Vec<i64>> = serde_json::from_value(config["phoneme_id_map"].clone()).expect("id map");
    let espeak_voice = config["espeak"]["voice"].as_str().expect("espeak voice").to_owned();
    let clusters: Option<BTreeSet<Vec<String>>> = config
        .get("vowel_clusters")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let g: Golden =
        serde_json::from_str(&std::fs::read_to_string(golden(&format!("phonemes_{voice}.json"))).expect("golden"))
            .expect("json");
    let mut same = 0;
    for row in &g.rows {
        let sentences = phonemize(espeak, &espeak_voice, &row.text, clusters.as_ref()).expect("phonemizes");
        let ids: Vec<Vec<i64>> = sentences
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| to_ids(s, &map).0)
            .collect();
        if ids == row.ids {
            same += 1;
        } else {
            let got: Vec<String> = sentences.iter().map(|s| s.concat()).collect();
            eprintln!(
                "DIFF {voice}: {}\n   piper: {:?}\n   rust:  {:?}",
                row.text, row.phonemes, got
            );
        }
    }
    (same, g.rows.len())
}

#[test]
#[ignore = "needs the voice configs (PB_WEIGHTS); run with --ignored"]
fn every_voice_matches_piper_exactly() {
    let mut espeak = Espeak::open(std::path::Path::new(pb_espeak::BUILD_DATA_DIR)).expect("espeak-ng starts");
    for voice in [
        "de_DE-thorsten-high",
        "de_DE-thorsten-medium",
        "en_US-lessac-high",
        "en_US-lessac-medium",
    ] {
        let (same, total) = compare(&mut espeak, voice);
        eprintln!("{voice}: {same}/{total} sentences identical");
        assert_eq!(same, total, "{voice}: {} sentences differ from Piper", total - same);
    }
}
