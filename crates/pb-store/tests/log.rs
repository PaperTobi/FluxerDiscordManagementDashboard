#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use jiff::Timestamp;
use pb_store::{Clock, JsonlLog};
use pb_store_api::{EventLog, NewEvent, StoreError, WriterHealth};

fn ev(i: u32) -> NewEvent {
    NewEvent {
        kind: "test".into(),
        v: 1,
        ts: None,
        data: serde_json::json!({ "i": i }),
    }
}

#[tokio::test]
async fn contract() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log");
    pb_store_api::contract::event_log(|| {
        let path = path.clone();
        async move { JsonlLog::open(&path).unwrap().0 }
    })
    .await;
}

#[tokio::test]
async fn torn_tail_is_cut_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    {
        let (log, repaired) = JsonlLog::open(dir.path()).unwrap();
        assert!(repaired.is_none());
        log.append(vec![ev(1), ev(2)]).await.unwrap();
        drop(log);
    }
    let seg = std::fs::read_dir(dir.path()).unwrap().next().unwrap().unwrap().path();
    let good = std::fs::read(&seg).unwrap();
    let mut torn = good.clone();
    torn.extend_from_slice(br#"{"seq":3,"ts":"2026-10-0"#);
    std::fs::write(&seg, &torn).unwrap();
    let (log, repaired) = JsonlLog::open(dir.path()).unwrap();
    let r = repaired.expect("repaired");
    assert_eq!((r.cut_bytes, r.last_good_seq), (24, 2));
    assert_eq!(std::fs::read(&seg).unwrap(), good);
    assert_eq!(log.append(vec![ev(3)]).await.unwrap()[0].seq, 3);
    assert!(log.verify().await.unwrap().ok());
}

#[tokio::test]
async fn a_damaged_last_line_is_not_guessed_away() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("000001-2026-10.jsonl"), b"{\"seq\":1,\"garbage\n").unwrap();
    assert!(matches!(JsonlLog::open(dir.path()), Err(StoreError::Corrupt(_))));
}

#[tokio::test]
async fn a_new_month_starts_a_new_segment_and_the_chain_continues() {
    let dir = tempfile::tempdir().unwrap();
    let now = Arc::new(Mutex::new("2026-10-31T23:59:59Z".parse::<Timestamp>().unwrap()));
    let clock: Clock = {
        let now = now.clone();
        Arc::new(move || *now.lock().unwrap())
    };
    let (log, _) = JsonlLog::open_with_clock(dir.path(), clock.clone()).unwrap();
    log.append(vec![ev(1)]).await.unwrap();
    *now.lock().unwrap() = "2026-11-01T00:00:01Z".parse().unwrap();
    log.append(vec![ev(2)]).await.unwrap();
    drop(log);
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["000001-2026-10.jsonl", "000002-2026-11.jsonl"]);
    let (log, _) = JsonlLog::open_with_clock(dir.path(), clock).unwrap();
    assert_eq!(log.head().map(|h| h.seq), Some(2));
    let all: Vec<_> = log.scan(1).map(|e| e.unwrap()).collect().await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].prev, all[0].hash);
    assert_eq!(log.scan(2).map(|e| e.unwrap().seq).collect::<Vec<_>>().await, vec![2]);
    assert!(log.verify().await.unwrap().ok());
}

/// A full disk: the write fails, nothing of the batch stays, the writer halts until asked to retry.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_full_disk_halts_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let month = {
        let d = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
        format!("{:04}-{:02}", d.year(), d.month())
    };
    let (log, _) = JsonlLog::open(dir.path()).unwrap();
    // The segment the writer is about to create is a device that is always full (only written, never read).
    std::os::unix::fs::symlink("/dev/full", dir.path().join(format!("000001-{month}.jsonl"))).unwrap();
    assert!(matches!(log.append(vec![ev(1)]).await, Err(StoreError::Io(_))));
    assert!(matches!(log.health(), WriterHealth::Halted { since_seq: 0, .. }));
    assert!(
        matches!(log.append(vec![ev(2)]).await, Err(StoreError::Halted(_))),
        "no automatic retry"
    );
    assert_eq!(log.head(), None);
    log.retry().await.unwrap();
    assert_eq!(log.health(), WriterHealth::Ok);
    assert!(
        matches!(log.append(vec![ev(3)]).await, Err(StoreError::Io(_))),
        "the disk is still full"
    );
}
