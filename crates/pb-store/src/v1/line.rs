//! One line of the event log: `{"seq":…,"ts":"…Z","kind":"…","v":1,"prev":"<sha256 of the previous line>","data":{…}}`.
//! The hash of a line is SHA-256 over its exact bytes without the newline; the first line's `prev` is `GENESIS`.

use jiff::Timestamp;
use pb_store_api::{LineHash, StoredEvent};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct LineOut<'a> {
    seq: u64,
    ts: String,
    kind: &'a str,
    v: u32,
    prev: LineHash,
    data: &'a serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineIn {
    seq: u64,
    ts: Timestamp,
    kind: String,
    v: u32,
    prev: LineHash,
    data: serde_json::Value,
}

/// Millisecond precision, UTC, `Z` suffix.
fn ts_text(ts: Timestamp) -> String {
    let ms = ts.as_millisecond();
    Timestamp::from_millisecond(ms).map_or_else(|_| ts.to_string(), |t| format!("{t:.3}"))
}

/// The line for an event (without the newline) and its hash.
pub fn encode_line(
    seq: u64,
    ts: Timestamp,
    kind: &str,
    v: u32,
    prev: LineHash,
    data: &serde_json::Value,
) -> (String, LineHash) {
    let line = serde_json::to_string(&LineOut {
        seq,
        ts: ts_text(ts),
        kind,
        v,
        prev,
        data,
    })
    .unwrap_or_else(|_| unreachable!("a JSON value always serializes"));
    let hash = LineHash::of(line.as_bytes());
    (line, hash)
}

/// Parses one line (without the newline); the error says what is wrong with it.
pub fn parse_line(line: &[u8]) -> Result<StoredEvent, String> {
    let l: LineIn = serde_json::from_slice(line).map_err(|e| format!("not an event line: {e}"))?;
    Ok(StoredEvent {
        seq: l.seq,
        ts: l.ts,
        kind: l.kind,
        v: l.v,
        prev: l.prev,
        data: l.data,
        hash: LineHash::of(line),
    })
}

/// Millisecond-precision timestamp, as stored.
pub fn round_ms(ts: Timestamp) -> Timestamp {
    Timestamp::from_millisecond(ts.as_millisecond()).unwrap_or(ts)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use pb_store_api::GENESIS;

    use super::*;

    #[test]
    fn lines_round_trip_and_chain() {
        let ts: Timestamp = "2026-10-04T12:00:00.123456Z".parse().unwrap_or_else(|_| unreachable!());
        let data = serde_json::json!({"b": 1, "a": [1, 2]});
        let (line, hash) = encode_line(7, ts, "sentence", 1, GENESIS, &data);
        assert!(
            line.starts_with(r#"{"seq":7,"ts":"2026-10-04T12:00:00.123Z","kind":"sentence","v":1,"prev":"0000"#),
            "{line}"
        );
        let ev = parse_line(line.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((ev.seq, ev.v, ev.hash, ev.prev), (7, 1, hash, GENESIS));
        assert_eq!(ev.ts, round_ms(ts));
        assert_eq!(ev.data, data);
        assert_eq!(LineHash::from_str(&hash.hex()), Ok(hash));
        assert!(parse_line(b"{\"seq\":1}").is_err());
    }
}
