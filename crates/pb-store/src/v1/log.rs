//! The event log on disk: `log/NNNNNN-YYYY-MM.jsonl` segments (a new one each month; the hash chain continues across
//! them). One writer thread does group commit: it drains every queued append, writes them with one `write_all`,
//! `sync_data`s, then answers. A failed write or sync (a full disk included) cuts the file back to the last good line
//! and halts the writer until an owner asks to try again (System page) or the bot restarts; nothing is retried by
//! itself.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use jiff::Timestamp;
use pb_store_api::{
    EventLog, EventRef, GENESIS, LineHash, LogRepaired, NewEvent, StoreError, StoredEvent, VerifyProblem, VerifyReport,
    WriterHealth,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::fsutil::sync_dir;
use super::line::{encode_line, parse_line, round_ms};

#[derive(Debug, Clone)]
struct Segment {
    number: u32,
    /// `YYYY-MM` of when the segment was started.
    month: String,
    path: PathBuf,
    /// The seq of its first line (`None` while empty).
    first_seq: Option<u64>,
    /// Bytes of complete, synced lines.
    len: u64,
}

impl Segment {
    fn name(number: u32, month: &str) -> String {
        format!("{number:06}-{month}.jsonl")
    }
}

#[derive(Debug)]
struct State {
    segments: Vec<Segment>,
    head: Option<EventRef>,
    health: WriterHealth,
}

type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

enum Cmd {
    Append(Vec<NewEvent>, Reply<Vec<EventRef>>),
    Retry(Reply<()>),
}

/// The JSONL event log.
#[derive(Debug, Clone)]
pub struct JsonlLog {
    state: Arc<Mutex<State>>,
    tx: mpsc::UnboundedSender<Cmd>,
    follow: broadcast::Sender<Arc<[StoredEvent]>>,
}

fn lock(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn month_of(ts: Timestamp) -> String {
    let d = ts.to_zoned(jiff::tz::TimeZone::UTC).date();
    format!("{:04}-{:02}", d.year(), d.month())
}

/// The last complete line of a file and where the torn tail (bytes after the last newline) starts.
fn tail(path: &Path) -> std::io::Result<(u64, Option<Vec<u8>>)> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if len == 0 {
        return Ok((0, None));
    }
    // Find the last newline, then the one before it.
    let mut end_nl: Option<u64> = None;
    let mut start_nl: Option<u64> = None;
    let mut pos = len;
    let mut buf = vec![0u8; 64 * 1024];
    'outer: while pos > 0 {
        let n = buf.len().min(usize::try_from(pos).unwrap_or(usize::MAX));
        pos -= n as u64;
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(&mut buf[..n])?;
        for i in (0..n).rev() {
            if buf[i] == b'\n' {
                let at = pos + i as u64;
                if end_nl.is_none() {
                    end_nl = Some(at);
                } else {
                    start_nl = Some(at);
                    break 'outer;
                }
            }
        }
    }
    let Some(end) = end_nl else { return Ok((0, None)) };
    let start = start_nl.map_or(0, |s| s + 1);
    let mut line = vec![0u8; usize::try_from(end - start).unwrap_or(0)];
    f.seek(SeekFrom::Start(start))?;
    f.read_exact(&mut line)?;
    Ok((end + 1, Some(line)))
}

fn first_seq(path: &Path) -> Result<Option<u64>, StoreError> {
    let f = File::open(path)?;
    let mut r = BufReader::new(f);
    let mut line = Vec::new();
    let n = r.read_until(b'\n', &mut line)?;
    if n == 0 || line.last() != Some(&b'\n') {
        return Ok(None);
    }
    line.pop();
    let ev =
        parse_line(&line).map_err(|e| StoreError::Corrupt(format!("the first line of {}: {e}", path.display())))?;
    Ok(Some(ev.seq))
}

impl JsonlLog {
    /// Opens (or creates) the log in `dir`. A torn tail is cut off and reported, to be recorded as `log.repaired`.
    pub fn open(dir: &Path) -> Result<(JsonlLog, Option<LogRepaired>), StoreError> {
        JsonlLog::open_with_clock(dir, Arc::new(Timestamp::now))
    }

    /// [`JsonlLog::open`] with the clock that stamps events and picks the month of new segments.
    pub fn open_with_clock(dir: &Path, clock: Clock) -> Result<(JsonlLog, Option<LogRepaired>), StoreError> {
        fs::create_dir_all(dir)?;
        let mut segments = list_segments(dir)?;
        let mut repaired = None;
        let mut head = None;
        let count = segments.len();
        for (i, seg) in segments.iter_mut().enumerate() {
            seg.len = fs::metadata(&seg.path)?.len();
            if i + 1 == count {
                let (good, last) = tail(&seg.path)?;
                if good < seg.len {
                    let f = OpenOptions::new().write(true).open(&seg.path)?;
                    f.set_len(good)?;
                    f.sync_all()?;
                    repaired = Some((
                        seg.path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        seg.len - good,
                    ));
                    seg.len = good;
                }
                if let Some(line) = last {
                    let ev = parse_line(&line)
                        .map_err(|e| StoreError::Corrupt(format!("the last line of {}: {e}", seg.path.display())))?;
                    head = Some(EventRef {
                        seq: ev.seq,
                        hash: ev.hash,
                    });
                }
            }
            seg.first_seq = first_seq(&seg.path)?;
        }
        // An empty last segment (just started): the head is the end of the one before.
        if head.is_none() && count >= 2 {
            let before = &segments[count - 2].path;
            let (_, last) = tail(before)?;
            if let Some(line) = last {
                let ev = parse_line(&line)
                    .map_err(|e| StoreError::Corrupt(format!("the last line of {}: {e}", before.display())))?;
                head = Some(EventRef {
                    seq: ev.seq,
                    hash: ev.hash,
                });
            }
        }
        let repaired = repaired.map(|(segment, cut_bytes)| LogRepaired {
            segment,
            cut_bytes,
            last_good_seq: head.map_or(0, |h| h.seq),
        });
        let state = Arc::new(Mutex::new(State {
            segments,
            head,
            health: WriterHealth::Ok,
        }));
        let (tx, rx) = mpsc::unbounded_channel();
        let (follow, _) = broadcast::channel(1024);
        let writer = Writer {
            dir: dir.to_owned(),
            state: state.clone(),
            follow: follow.clone(),
            file: None,
            clock,
        };
        std::thread::Builder::new()
            .name("pb-log-writer".into())
            .spawn(move || writer.run(rx))?;
        Ok((JsonlLog { state, tx, follow }, repaired))
    }

    fn snapshot(&self) -> Vec<Segment> {
        lock(&self.state).segments.clone()
    }
}

/// The segment files of a log directory, in order (lengths and first lines not read yet).
fn list_segments(dir: &Path) -> Result<Vec<Segment>, StoreError> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".jsonl") else {
            continue;
        };
        let Some((num, month)) = stem.split_once('-') else {
            continue;
        };
        let Ok(number) = num.parse::<u32>() else { continue };
        if num.len() != 6 || month.len() != 7 {
            continue;
        }
        segments.push(Segment {
            number,
            month: month.to_owned(),
            path: entry.path(),
            first_seq: None,
            len: 0,
        });
    }
    segments.sort_by_key(|s| s.number);
    Ok(segments)
}

/// Checks the hash chain of the log in `dir` without changing anything: safe while the bot runs (a line being
/// written at the end is not looked at).
pub fn verify_dir(dir: &Path) -> Result<VerifyReport, StoreError> {
    let mut segments = list_segments(dir)?;
    let count = segments.len();
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.len = if i + 1 == count {
            tail(&seg.path)?.0
        } else {
            fs::metadata(&seg.path)?.len()
        };
    }
    verify(&segments)
}

struct Writer {
    dir: PathBuf,
    state: Arc<Mutex<State>>,
    follow: broadcast::Sender<Arc<[StoredEvent]>>,
    /// The open last segment.
    file: Option<(u32, File)>,
    clock: Clock,
}

/// Where the log gets the time.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

impl Writer {
    /// Writes until every handle to the log is gone. Appends queued together are committed together; a retry
    /// waits for the appends before it.
    fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        while let Some(cmd) = rx.blocking_recv() {
            match cmd {
                Cmd::Append(events, reply) => {
                    let mut batch = vec![(events, reply)];
                    let mut retry = None;
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            Cmd::Append(e, r) => batch.push((e, r)),
                            Cmd::Retry(r) => {
                                retry = Some(r);
                                break;
                            }
                        }
                    }
                    self.commit(batch);
                    if let Some(reply) = retry {
                        self.retry(reply);
                    }
                }
                Cmd::Retry(reply) => self.retry(reply),
            }
        }
    }

    /// After a halt: the next commit writes again (from the last good line).
    fn retry(&mut self, reply: Reply<()>) {
        lock(&self.state).health = WriterHealth::Ok;
        let _ = reply.send(Ok(()));
    }

    fn commit(&mut self, batch: Vec<(Vec<NewEvent>, Reply<Vec<EventRef>>)>) {
        let halted = match &lock(&self.state).health {
            WriterHealth::Halted { error, .. } => Some(error.clone()),
            WriterHealth::Ok => None,
        };
        if let Some(error) = halted {
            for (_, reply) in batch {
                let _ = reply.send(Err(StoreError::Halted(error.clone())));
            }
            return;
        }
        let now = (self.clock)();
        let (mut seq, mut prev) = lock(&self.state).head.map_or((0, GENESIS), |h| (h.seq, h.hash));
        let mut text = String::new();
        let mut stored = Vec::new();
        let mut answers = Vec::new();
        for (events, reply) in batch {
            let mut refs = Vec::with_capacity(events.len());
            for ev in events {
                seq += 1;
                let ts = round_ms(ev.ts.unwrap_or(now));
                let (line, hash) = encode_line(seq, ts, &ev.kind, ev.v, prev, &ev.data);
                text.push_str(&line);
                text.push('\n');
                stored.push(StoredEvent {
                    seq,
                    ts,
                    kind: ev.kind,
                    v: ev.v,
                    prev,
                    data: ev.data,
                    hash,
                });
                refs.push(EventRef { seq, hash });
                prev = hash;
            }
            answers.push((reply, refs));
        }
        if stored.is_empty() {
            for (reply, refs) in answers {
                let _ = reply.send(Ok(refs));
            }
            return;
        }
        match self.write(&text, month_of(now), stored.first().map_or(1, |e| e.seq)) {
            Ok(()) => {
                let head = stored.last().map(|e| EventRef {
                    seq: e.seq,
                    hash: e.hash,
                });
                lock(&self.state).head = head;
                let _ = self.follow.send(stored.into());
                for (reply, refs) in answers {
                    let _ = reply.send(Ok(refs));
                }
            }
            Err(e) => {
                let error = e.to_string();
                tracing::error!(%error, "the event log stopped writing");
                {
                    let mut st = lock(&self.state);
                    let since_seq = st.head.map_or(0, |h| h.seq);
                    st.health = WriterHealth::Halted {
                        error: error.clone(),
                        since_seq,
                    };
                }
                for (reply, _) in answers {
                    let _ = reply.send(Err(StoreError::Io(error.clone())));
                }
            }
        }
    }

    /// Appends `text` to the current segment (starting a new one when the month changed), synced. On failure the
    /// file is cut back to its last good length.
    fn write(&mut self, text: &str, month: String, first_seq: u64) -> std::io::Result<()> {
        let current = {
            let st = lock(&self.state);
            st.segments
                .last()
                .filter(|s| s.month == month || s.len == 0)
                .map(|s| (s.number, s.path.clone(), s.len))
        };
        let (number, path, len) = match current {
            Some(c) => c,
            // A new month: the segment counts only once its file and directory entry are durable.
            None => {
                let number = lock(&self.state).segments.last().map_or(1, |s| s.number + 1);
                let path = self.dir.join(Segment::name(number, &month));
                File::create_new(&path).or_else(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => File::open(&path),
                    _ => Err(e),
                })?;
                sync_dir(&self.dir)?;
                lock(&self.state).segments.push(Segment {
                    number,
                    month,
                    path: path.clone(),
                    first_seq: None,
                    len: 0,
                });
                (number, path, 0)
            }
        };
        let file = match self.file.take() {
            Some((n, f)) if n == number => f,
            // (Re)opening: everything past the last good line goes (what a failed write may have left), and writing
            // continues right there.
            _ => {
                let mut f = OpenOptions::new().write(true).open(&path)?;
                f.set_len(len)?;
                f.sync_all()?;
                f.seek(SeekFrom::Start(len))?;
                f
            }
        };
        let (_, f) = self.file.insert((number, file));
        let result = f.write_all(text.as_bytes()).and_then(|()| f.sync_data());
        match result {
            Ok(()) => {
                let mut st = lock(&self.state);
                if let Some(seg) = st.segments.iter_mut().find(|s| s.number == number) {
                    seg.len = len + text.len() as u64;
                    seg.first_seq.get_or_insert(first_seq);
                }
                Ok(())
            }
            Err(e) => {
                // Cut back to the last good line now; whatever stays is cut when the file is opened again.
                let _ = f.set_len(len).and_then(|()| f.sync_all());
                self.file = None;
                Err(e)
            }
        }
    }
}

/// Reads complete lines of the given segments (up to their committed lengths) from `from_seq` on.
fn read_lines(segments: Vec<Segment>, from_seq: u64, tx: mpsc::Sender<Result<StoredEvent, StoreError>>) {
    let start = segments
        .iter()
        .rposition(|s| s.first_seq.is_some_and(|f| f <= from_seq))
        .unwrap_or(0);
    for seg in segments.into_iter().skip(start) {
        let file = match File::open(&seg.path) {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.blocking_send(Err(e.into()));
                return;
            }
        };
        let mut r = BufReader::new(file.take(seg.len));
        let mut line = Vec::new();
        let mut number = 0u64;
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.blocking_send(Err(e.into()));
                    return;
                }
            }
            if line.last() != Some(&b'\n') {
                break;
            }
            line.pop();
            number += 1;
            let item = parse_line(&line)
                .map_err(|e| StoreError::Corrupt(format!("line {number} of {}: {e}", seg.path.display())));
            if let Ok(ev) = &item
                && ev.seq < from_seq
            {
                continue;
            }
            let stop = item.is_err();
            if tx.blocking_send(item).is_err() || stop {
                return;
            }
        }
    }
}

#[async_trait]
impl EventLog for JsonlLog {
    async fn append(&self, events: Vec<NewEvent>) -> Result<Vec<EventRef>, StoreError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Append(events, tx))
            .map_err(|_| StoreError::Io("the log writer has stopped".into()))?;
        rx.await
            .map_err(|_| StoreError::Io("the log writer has stopped".into()))?
    }

    fn head(&self) -> Option<EventRef> {
        lock(&self.state).head
    }

    fn health(&self) -> WriterHealth {
        lock(&self.state).health.clone()
    }

    async fn retry(&self) -> Result<(), StoreError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Retry(tx))
            .map_err(|_| StoreError::Io("the log writer has stopped".into()))?;
        rx.await
            .map_err(|_| StoreError::Io("the log writer has stopped".into()))?
    }

    fn scan(&self, from_seq: u64) -> BoxStream<'_, Result<StoredEvent, StoreError>> {
        let segments = self.snapshot();
        let (tx, rx) = mpsc::channel(1024);
        tokio::task::spawn_blocking(move || read_lines(segments, from_seq.max(1), tx));
        stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) }).boxed()
    }

    async fn verify(&self) -> Result<VerifyReport, StoreError> {
        let segments = self.snapshot();
        tokio::task::spawn_blocking(move || verify(&segments))
            .await
            .map_err(|e| StoreError::Io(e.to_string()))?
    }

    fn follow(&self) -> broadcast::Receiver<Arc<[StoredEvent]>> {
        self.follow.subscribe()
    }

    fn size(&self) -> u64 {
        lock(&self.state).segments.iter().map(|s| s.len).sum()
    }
}

fn verify(segments: &[Segment]) -> Result<VerifyReport, StoreError> {
    let mut report = VerifyReport::default();
    let mut expect_seq = 1u64;
    let mut prev = GENESIS;
    for seg in segments {
        report.segments += 1;
        let name = seg
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut r = BufReader::new(File::open(&seg.path)?.take(seg.len));
        let mut line = Vec::new();
        let mut n = 0u64;
        loop {
            line.clear();
            if r.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            n += 1;
            report.bytes += line.len() as u64;
            if line.last() != Some(&b'\n') {
                report.problems.push(VerifyProblem {
                    segment: name.clone(),
                    line: n,
                    message: "unfinished last line".into(),
                });
                break;
            }
            line.pop();
            match parse_line(&line) {
                Ok(ev) => {
                    report.events += 1;
                    if ev.seq != expect_seq {
                        report.problems.push(VerifyProblem {
                            segment: name.clone(),
                            line: n,
                            message: format!("seq {} where {expect_seq} was expected", ev.seq),
                        });
                    }
                    if ev.prev != prev {
                        report.problems.push(VerifyProblem {
                            segment: name.clone(),
                            line: n,
                            message: format!("event {}: prev does not match the hash of the line before", ev.seq),
                        });
                    }
                    expect_seq = ev.seq + 1;
                    prev = ev.hash;
                }
                Err(e) => {
                    report.problems.push(VerifyProblem {
                        segment: name.clone(),
                        line: n,
                        message: e.to_string(),
                    });
                    // Keep checking the chain from the next line on.
                    prev = LineHash::of(&line);
                    expect_seq += 1;
                }
            }
        }
    }
    Ok(report)
}
