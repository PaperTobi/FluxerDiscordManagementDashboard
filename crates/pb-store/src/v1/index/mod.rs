//! The Turso query index, derived from the event log: it follows the log and can always be rebuilt from it.

mod apply;
mod db;
mod query;
mod schema;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use jiff::Timestamp;
use jiff::civil::Date;
use pb_domain::{GuildId, SentenceId, UserId};
use pb_store_api::{
    ActionRecord, AuditFilter, AuditRow, ClipRow, CommunitySeen, Cursor, DayRow, DigestRow, EventLog, Index, JarRow,
    LastDigest, Page, PersonName, SentenceFilter, SentenceRow, StoreError, StoredEvent,
};
use tokio::sync::{broadcast, mpsc, watch};

use db::{Db, Work, call};

/// How many events go into one index transaction while catching up.
const CATCH_UP_BATCH: usize = 1000;

/// The index.
#[derive(Debug, Clone)]
pub struct TursoIndex {
    jobs: mpsc::UnboundedSender<Work>,
    applied: watch::Receiver<u64>,
    error: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for Work {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Work")
    }
}

async fn apply_batch(db: &Db, events: &[StoredEvent]) -> Result<u64, StoreError> {
    let Some(last) = events.last() else { return Ok(0) };
    db.batch("BEGIN").await?;
    let result = async {
        for e in events {
            apply::apply(db, e).await?;
        }
        db.set_meta("applied_seq", &last.seq.to_string()).await?;
        db.set_meta("applied_hash", &last.hash.hex()).await
    }
    .await;
    match result {
        Ok(()) => match db.batch("COMMIT").await {
            Ok(()) => Ok(last.seq),
            // Left open, the transaction would make every later BEGIN fail.
            Err(e) => {
                let _ = db.batch("ROLLBACK").await;
                Err(e)
            }
        },
        Err(e) => {
            let _ = db.batch("ROLLBACK").await;
            Err(e)
        }
    }
}

impl TursoIndex {
    /// Opens the index at `path` and starts following `log`. An index that does not match the log (another log, a
    /// newer index, another schema) is rebuilt from scratch.
    pub async fn open(path: &Path, log: Arc<dyn EventLog>) -> Result<TursoIndex, StoreError> {
        let jobs = db::spawn(path).await?;
        let (applied_seq, applied_hash) = call(&jobs, |db| {
            Box::pin(async move { Ok((db.meta("applied_seq").await?, db.meta("applied_hash").await?)) })
        })
        .await?;
        let mut applied = applied_seq.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        if applied > 0 {
            let matches = match log.scan(applied).next().await {
                Some(Ok(e)) => e.seq == applied && Some(e.hash.hex()) == applied_hash,
                _ => false,
            };
            if !matches {
                tracing::info!("the index does not match the event log; rebuilding it");
                db::rebuild(&jobs).await?;
                applied = 0;
            }
        }
        let (tx, rx) = watch::channel(applied);
        let error = Arc::new(Mutex::new(None));
        tokio::spawn(follow(log, jobs.clone(), tx, error.clone()));
        Ok(TursoIndex {
            jobs,
            applied: rx,
            error,
        })
    }
}

/// Catches up with the log, then applies what it commits; a lag or an error means catching up again.
async fn follow(
    log: Arc<dyn EventLog>,
    jobs: mpsc::UnboundedSender<Work>,
    applied: watch::Sender<u64>,
    error: Arc<Mutex<Option<String>>>,
) {
    let mut rx = log.follow();
    // Applying again means whatever went wrong before is over.
    let progressed = |seq: u64| {
        applied.send_replace(seq);
        if let Ok(mut e) = error.lock() {
            *e = None;
        }
    };
    loop {
        let result = async {
            let mut stream = log.scan(*applied.borrow() + 1);
            let mut batch = Vec::with_capacity(CATCH_UP_BATCH);
            while let Some(e) = stream.next().await {
                batch.push(e?);
                if batch.len() >= CATCH_UP_BATCH {
                    let events = std::mem::take(&mut batch);
                    let seq = call(&jobs, move |db| Box::pin(async move { apply_batch(db, &events).await })).await?;
                    progressed(seq);
                }
            }
            if !batch.is_empty() {
                let seq = call(&jobs, move |db| Box::pin(async move { apply_batch(db, &batch).await })).await?;
                progressed(seq);
            }
            loop {
                match rx.recv().await {
                    Ok(committed) => {
                        let have = *applied.borrow();
                        let fresh: Vec<StoredEvent> = committed.iter().filter(|e| e.seq > have).cloned().collect();
                        let Some(first) = fresh.first() else { continue };
                        if first.seq != have + 1 {
                            return Ok(true); // a gap: catch up from the log
                        }
                        // A big append (an import) goes in in pieces, one transaction each.
                        for piece in fresh.chunks(CATCH_UP_BATCH) {
                            let piece = piece.to_vec();
                            let seq =
                                call(&jobs, move |db| Box::pin(async move { apply_batch(db, &piece).await })).await?;
                            progressed(seq);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => return Ok(true),
                    Err(broadcast::error::RecvError::Closed) => return Ok::<bool, StoreError>(false),
                }
            }
        }
        .await;
        match result {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::error!(error = %e, "the index could not apply events; trying again in 5 s");
                if let Ok(mut slot) = error.lock() {
                    *slot = Some(e.to_string());
                }
                if jobs.is_closed() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

#[async_trait]
impl Index for TursoIndex {
    async fn caught_up(&self, seq: u64) {
        let mut rx = self.applied.clone();
        let _ = rx.wait_for(|a| *a >= seq).await;
    }

    fn problem(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    async fn skipped(&self) -> Result<u64, StoreError> {
        call(&self.jobs, |db| {
            Box::pin(async move { Ok(db.meta("skipped").await?.and_then(|s| s.parse().ok()).unwrap_or(0)) })
        })
        .await
    }

    fn applied(&self) -> u64 {
        *self.applied.borrow()
    }

    async fn sentences(
        &self,
        f: &SentenceFilter,
        cursor: Option<Cursor>,
        page: u32,
    ) -> Result<Page<SentenceRow>, StoreError> {
        let f = f.clone();
        call(&self.jobs, move |db| Box::pin(query::sentences(db, f, cursor, page))).await
    }

    async fn sentence(&self, id: SentenceId) -> Result<Option<SentenceRow>, StoreError> {
        call(&self.jobs, move |db| Box::pin(query::sentence(db, id))).await
    }

    async fn days(
        &self,
        guild: GuildId,
        user: UserId,
        from: Date,
        until: Date,
        tz: &str,
    ) -> Result<Vec<DayRow>, StoreError> {
        let tz = tz.to_owned();
        call(&self.jobs, move |db| {
            Box::pin(query::days(db, guild, user, from, until, tz))
        })
        .await
    }

    async fn jar(&self, guild: Option<GuildId>) -> Result<Vec<JarRow>, StoreError> {
        call(&self.jobs, move |db| Box::pin(query::jar(db, guild))).await
    }

    async fn violation_times(&self) -> Result<Vec<(GuildId, UserId, Timestamp)>, StoreError> {
        call(&self.jobs, |db| Box::pin(query::violation_times(db))).await
    }

    async fn pending_undos(&self) -> Result<Vec<ActionRecord>, StoreError> {
        call(&self.jobs, |db| Box::pin(query::pending_undos(db))).await
    }

    async fn audit(&self, f: &AuditFilter, cursor: Option<Cursor>, page: u32) -> Result<Page<AuditRow>, StoreError> {
        let f = f.clone();
        call(&self.jobs, move |db| Box::pin(query::audit(db, f, cursor, page))).await
    }

    async fn clips(&self) -> Result<Vec<ClipRow>, StoreError> {
        call(&self.jobs, |db| Box::pin(query::clips(db))).await
    }

    async fn people(&self, users: &[UserId], guild: Option<GuildId>) -> Result<Vec<PersonName>, StoreError> {
        let users = users.to_vec();
        call(&self.jobs, move |db| Box::pin(query::people(db, users, guild))).await
    }

    async fn communities(&self) -> Result<Vec<CommunitySeen>, StoreError> {
        call(&self.jobs, |db| Box::pin(query::communities(db))).await
    }

    async fn last_digest(&self) -> Result<LastDigest, StoreError> {
        call(&self.jobs, |db| Box::pin(query::last_digest(db))).await
    }

    async fn digest(&self, from: Timestamp, until: Timestamp) -> Result<Vec<DigestRow>, StoreError> {
        call(&self.jobs, move |db| Box::pin(query::digest(db, from, until))).await
    }
}
