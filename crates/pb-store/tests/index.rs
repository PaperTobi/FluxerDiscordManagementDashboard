#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::sync::Arc;

use pb_store::{JsonlLog, TursoIndex};
use pb_store_api::{EventLog, Index, NewEvent};

#[tokio::test(flavor = "multi_thread")]
async fn contract() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    pb_store_api::contract::index(|| {
        let path = path.clone();
        async move {
            let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&path.join("log")).unwrap().0);
            let index = TursoIndex::open(&path.join("index/index.db"), log.clone())
                .await
                .unwrap();
            (log, index)
        }
    })
    .await;
}

fn ev(i: u32) -> NewEvent {
    NewEvent {
        kind: "test".into(),
        v: 1,
        ts: None,
        data: serde_json::json!({ "i": i }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_where_it_stopped_and_rebuilds_for_another_log() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index/index.db");
    {
        let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&dir.path().join("log")).unwrap().0);
        log.append((0..2500).map(ev).collect()).await.unwrap();
        let index = TursoIndex::open(&db, log.clone()).await.unwrap();
        index.caught_up(2500).await;
        assert_eq!(index.applied(), 2500);
    }
    {
        let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&dir.path().join("log")).unwrap().0);
        log.append(vec![ev(1)]).await.unwrap();
        let index = TursoIndex::open(&db, log.clone()).await.unwrap();
        assert!(index.applied() >= 2500, "resumes instead of starting over");
        index.caught_up(2501).await;
    }
    // Another log (say, restored from elsewhere): the index notices and starts over.
    let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&dir.path().join("other-log")).unwrap().0);
    log.append(vec![ev(7), ev(8)]).await.unwrap();
    let index = TursoIndex::open(&db, log.clone()).await.unwrap();
    index.caught_up(2).await;
    assert_eq!(index.applied(), 2);
    assert_eq!(index.applied(), 2);
}
