//! Parity with the old Python bot: one person's speech through the whole bot (voice in, Silero VAD, the segmenter, the
//! Roblox classifier on the CPU, the decider) is cut, scored and decided exactly as the old bot's own per-track pipeline
//! and decider did it (recorded in `tests/golden/parity.json` by `tools/golden/oracle.py --only parity`, a script now in the history at 18db450): the same
//! samples in every sentence, scores within 1e-4, the same flagged types and decisions. Every detection type is on, two
//! strikes, observe-only. The audio arrives sample-exact through the in-process transport (a codec would change it).
//! Needs the model weights (`PB_WEIGHTS`); speaks in real time (~50 s).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use std::path::PathBuf;

use common::{ALICE, G, Rig, Setup, VOICE};
use pb_domain::{ChannelId, GuildId, Label, Scope, UserId};
use pb_settings::SettingKey;
use pb_store_api::{BlobStore, DecisionRecord, Event, SentenceRecord};
use pb_testkit::memvoice::MemVoice;
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    labels: Vec<String>,
    sentences: Vec<Sentence>,
}

#[derive(Deserialize)]
struct Sentence {
    s0: usize,
    s1: usize,
    scores: Vec<f32>,
    flagged: Vec<String>,
    /// The old decider: `skip` (with `reason` "strike k of n" while strikes count up) or `observe`.
    decision: String,
    reason: String,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The old decider's words for a decision.
fn old_words(d: &DecisionRecord) -> (String, String) {
    match d {
        DecisionRecord::NothingFlagged => ("skip".into(), String::new()),
        DecisionRecord::Strike { strike, of } => ("skip".into(), format!("strike {strike} of {of}")),
        DecisionRecord::Observe { .. } => ("observe".into(), "observe-only".into()),
        other => (format!("{other:?}"), String::new()),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the model weights (PB_WEIGHTS) and speaks in real time"]
async fn cuts_scores_and_decides_like_the_old_bot() {
    let golden: Golden =
        serde_json::from_str(&std::fs::read_to_string(golden_dir().join("parity.json")).unwrap()).unwrap();
    let (stream, rate) = pb_testkit::audio::read_wav(&golden_dir().join("parity.wav")).unwrap();
    assert_eq!(rate, 16_000);
    assert_eq!(
        golden.labels,
        Label::ALL.iter().map(|l| l.model_name()).collect::<Vec<_>>(),
        "label order"
    );

    let voice = MemVoice::default();
    let rig = Rig::start(Setup {
        memory_voice: Some(voice.clone()),
        settings: Box::new(|tree| {
            let mut set = |key, value| tree.set(Scope::Global, key, value, true).unwrap();
            let mut changes: Vec<_> = Label::ALL
                .iter()
                .filter_map(|l| set(SettingKey::LabelEnabled(*l), serde_json::json!(true)))
                .collect();
            changes.extend(set(SettingKey::Strikes, serde_json::json!(2)));
            changes.extend(set(SettingKey::ObserveOnly, serde_json::json!(true)));
            changes.extend(set(SettingKey::Recordings, serde_json::json!("all")));
            changes
        }),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins_memory(&voice).await;
    mic.say(&stream).await;

    let want = golden.sentences.len();
    let sentences = || async {
        let mut got: Vec<SentenceRecord> = rig
            .events()
            .await
            .into_iter()
            .filter_map(|e| match e {
                Event::Sentence(s) if s.user == UserId(ALICE) => Some(*s),
                _ => None,
            })
            .collect();
        got.sort_by_key(|s| s.started);
        got
    };
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let got = loop {
        let got = sentences().await;
        if got.len() >= want || std::time::Instant::now() > end {
            break got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    assert_eq!(
        got.len(),
        want,
        "sentences: {:?}",
        got.iter().map(|s| s.dur_ms).collect::<Vec<_>>()
    );

    for (i, (g, w)) in got.iter().zip(&golden.sentences).enumerate() {
        let path = rig
            .blobs
            .path(g.audio.as_ref().expect("every recording is kept"))
            .await
            .unwrap();
        let (pcm, _) = pb_testkit::audio::read_wav(&path).unwrap();
        assert!(
            pcm.as_slice() == &stream[w.s0..w.s1],
            "sentence {i}: the old bot cut samples {}..{} ({} samples), this one {} samples",
            w.s0,
            w.s1,
            w.s1 - w.s0,
            pcm.len()
        );
        let worst = g
            .scores
            .iter()
            .zip(&w.scores)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            worst <= 1e-4,
            "sentence {i}: scores differ by {worst}: {:?} vs {:?}",
            g.scores,
            w.scores
        );
        let mut flagged: Vec<&str> = g.flagged.iter().map(|l| l.model_name()).collect();
        let mut want_flagged: Vec<&str> = w.flagged.iter().map(String::as_str).collect();
        flagged.sort_unstable();
        want_flagged.sort_unstable();
        assert_eq!(flagged, want_flagged, "sentence {i}: flagged types");
        assert_eq!(
            old_words(&g.decision),
            (w.decision.clone(), w.reason.clone()),
            "sentence {i}: decision"
        );
    }
    assert!(
        voice.played(GuildId(G), ChannelId(VOICE)).is_empty(),
        "observe-only: the bot stays silent"
    );
    rig.stop(None).await;
}
