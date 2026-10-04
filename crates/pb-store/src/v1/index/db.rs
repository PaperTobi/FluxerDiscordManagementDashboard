//! The index database, owned by one thread (Turso does its file I/O inside its futures, so it gets its own runtime
//! and never blocks the bot's).

use std::path::{Path, PathBuf};

use futures::future::LocalBoxFuture;
use pb_store_api::StoreError;
use tokio::sync::{mpsc, oneshot};
use turso::Value;

use super::schema::{DDL, SCHEMA};

pub fn ix(e: turso::Error) -> StoreError {
    StoreError::Index(e.to_string())
}

/// One connection with small helpers.
pub struct Db {
    conn: turso::Connection,
}

impl Db {
    pub async fn exec(&self, sql: &str, params: Vec<Value>) -> Result<u64, StoreError> {
        self.conn.execute(sql, params).await.map_err(ix)
    }

    pub async fn batch(&self, sql: &str) -> Result<(), StoreError> {
        self.conn.execute_batch(sql).await.map_err(ix)
    }

    pub async fn rows(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, StoreError> {
        let mut rows = self.conn.query(sql, params).await.map_err(ix)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(ix)? {
            let mut cols = Vec::with_capacity(row.column_count());
            for i in 0..row.column_count() {
                cols.push(row.get_value(i).map_err(ix)?);
            }
            out.push(cols);
        }
        Ok(out)
    }

    pub async fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        let rows = self
            .rows("SELECT value FROM meta WHERE key = ?1", vec![key.into()])
            .await?;
        Ok(rows.into_iter().next().and_then(|r| text(&r[0])))
    }

    pub async fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            vec![key.into(), value.into()],
        )
        .await
        .map(|_| ())
    }
}

pub fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

pub fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        Value::Real(f) => Some(*f as i64),
        Value::Text(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn opt<T: Into<Value>>(v: Option<T>) -> Value {
    v.map_or(Value::Null, Into::into)
}

/// A closure run on the index thread.
pub type Job = Box<dyn for<'c> FnOnce(&'c Db) -> LocalBoxFuture<'c, ()> + Send>;

/// Work for the index thread.
pub enum Work {
    Run(Job),
    /// Drop every table and start empty (the follower then replays the log).
    Rebuild(oneshot::Sender<Result<(), StoreError>>),
}

/// Every file of the database (the main file and its side files).
fn files(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else { return Vec::new() };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&name))
        .map(|e| e.path())
        .collect()
}

pub fn delete_files(path: &Path) -> Result<(), StoreError> {
    for f in files(path) {
        std::fs::remove_file(&f)?;
    }
    Ok(())
}

async fn open_db(path: &Path) -> Result<Db, StoreError> {
    let p = path
        .to_str()
        .ok_or_else(|| StoreError::Index(format!("{} is not UTF-8", path.display())))?;
    let db = turso::Builder::new_local(p).build().await.map_err(ix)?;
    let conn = db.connect().map_err(ix)?;
    let db = Db { conn };
    db.batch(DDL).await?;
    Ok(db)
}

/// Opens the database (recreating it when its schema is not this build's) on its own thread.
pub async fn spawn(path: &Path) -> Result<mpsc::UnboundedSender<Work>, StoreError> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Work>();
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), StoreError>>();
    let path = path.to_owned();
    std::thread::Builder::new().name("pb-index".into()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = ready_tx.send(Err(StoreError::Io(e.to_string())));
                return;
            }
        };
        rt.block_on(async move {
            let opened = async {
                let mut db = open_db(&path).await?;
                let schema = db.meta("schema").await?;
                if schema.as_deref() != Some(&SCHEMA.to_string()) {
                    if schema.is_some() {
                        tracing::info!("the index was built by another version; rebuilding it from the log");
                        drop(db);
                        delete_files(&path)?;
                        db = open_db(&path).await?;
                    }
                    db.set_meta("schema", &SCHEMA.to_string()).await?;
                }
                Ok::<Db, StoreError>(db)
            }
            .await;
            let mut db = match opened {
                Ok(db) => {
                    let _ = ready_tx.send(Ok(()));
                    db
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            while let Some(work) = rx.recv().await {
                match work {
                    Work::Run(job) => job(&db).await,
                    Work::Rebuild(done) => {
                        drop(db);
                        let reopened = async {
                            delete_files(&path)?;
                            let db = open_db(&path).await?;
                            db.set_meta("schema", &SCHEMA.to_string()).await?;
                            Ok::<Db, StoreError>(db)
                        }
                        .await;
                        match reopened {
                            Ok(d) => {
                                db = d;
                                let _ = done.send(Ok(()));
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "could not recreate the index");
                                let _ = done.send(Err(e));
                                return;
                            }
                        }
                    }
                }
            }
        });
    })?;
    ready_rx
        .await
        .map_err(|_| StoreError::Index("the index thread stopped".into()))??;
    Ok(tx)
}

/// Runs `f` on the index thread.
pub async fn call<T, F>(jobs: &mpsc::UnboundedSender<Work>, f: F) -> Result<T, StoreError>
where
    T: Send + 'static,
    F: for<'c> FnOnce(&'c Db) -> LocalBoxFuture<'c, Result<T, StoreError>> + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    let job: Job = Box::new(move |db| {
        Box::pin(async move {
            let _ = tx.send(f(db).await);
        })
    });
    jobs.send(Work::Run(job))
        .map_err(|_| StoreError::Index("the index thread stopped".into()))?;
    rx.await
        .map_err(|_| StoreError::Index("the index thread stopped".into()))?
}

/// Empties the index.
pub async fn rebuild(jobs: &mpsc::UnboundedSender<Work>) -> Result<(), StoreError> {
    let (tx, rx) = oneshot::channel();
    jobs.send(Work::Rebuild(tx))
        .map_err(|_| StoreError::Index("the index thread stopped".into()))?;
    rx.await
        .map_err(|_| StoreError::Index("the index thread stopped".into()))?
}
