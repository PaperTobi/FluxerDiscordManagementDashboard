//! The segmenter against the old bot's Python segmenter: the event traces `tools/golden/oracle.py` (a script now in the history at 18db450) recorded for the
//! Silero probabilities of real speech, noise and dialogue, with the bot's default and a fast configuration.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::path::PathBuf;

use pb_segment::{Event, FlushReason, SegCfg, Segmenter};
use safetensors::SafeTensors;
use serde_json::{Value, json};

fn probs(name: &str) -> Vec<f32> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../pb-vad-silero/tests/golden/{name}.safetensors"));
    let bytes = std::fs::read(path).expect("vad golden");
    let st = SafeTensors::deserialize(&bytes).expect("safetensors");
    pb_testkit::golden::f32s(st.tensor("probs").expect("probs").data())
}

fn to_json(frame: i64, ev: &Event) -> Value {
    match ev {
        Event::Open(o) => json!({"frame": frame, "type": "Open", "id": o.id, "s0": o.s0, "continues": o.continues}),
        Event::Cut(c) => {
            json!({"frame": frame, "type": "Cut", "id": c.id, "s0": c.s0, "s1": c.s1, "v0": c.v0, "v1": c.v1,
                                 "reason": serde_json::to_value(c.reason).expect("reason"), "voiced_ms": c.voiced_ms, "now": c.now})
        }
        Event::Drop(d) => {
            json!({"frame": frame, "type": "Drop", "id": d.id, "s0": d.s0, "s1": d.s1, "reason": "too_short"})
        }
        Event::Blip { s0, s1 } => json!({"frame": frame, "type": "Blip", "s0": s0, "s1": s1}),
    }
}

#[test]
fn reproduces_the_python_segmenter_exactly() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/traces.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("traces")).expect("json");
    let traces = all["traces"].as_object().expect("traces");
    assert!(traces.len() >= 6);
    for (key, trace) in traces {
        let name = key.split('/').next().expect("name");
        let cfg: SegCfg = serde_json::from_value(trace["cfg"].clone()).expect("cfg");
        let mut next = 0u64;
        let mut seg = Segmenter::new(
            cfg,
            || {
                next += 1;
                next
            },
            0,
        );
        let p = probs(name);
        let mut got = Vec::new();
        for (k, prob) in p.iter().enumerate() {
            for ev in seg.push(k as i64, f64::from(*prob)) {
                got.push(to_json(k as i64, &ev));
            }
        }
        let n = p.len() as i64;
        for ev in seg.flush(n * 512, FlushReason::End) {
            got.push(to_json(n, &ev));
        }
        let want = trace["events"].as_array().expect("events");
        assert!(!want.is_empty() || name == "noise", "{key}: the trace has events");
        assert_eq!(&got, want, "{key}: events differ");
        eprintln!("{key}: {} events identical", got.len());
    }
}
