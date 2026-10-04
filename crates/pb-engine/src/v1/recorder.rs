//! The single writer of the event log. Everyone hands events to the recorder and goes on: they are written in the
//! order they were handed over, as few appends as possible (whatever queued while the last append was being synced
//! goes in the next one), so no actor waits for the disk in the middle of its work. Whoever must know that events are
//! on disk (a clip that may only be used once it is durable, a login, the stop) asks for an acknowledgement.

use std::sync::Arc;

use bytes::Bytes;
use pb_store_api::{BlobAdded, BlobRole, BlobStore, Event, EventLog, NewEvent, SentenceRecord, StoreError};
use tokio::sync::oneshot;

use super::mailbox::{Addr, Mailbox};
use super::supervise::{ActorError, Life, Policy, Supervised};

pub(crate) enum RecordMsg {
    Events {
        events: Vec<NewEvent>,
        ack: Option<oneshot::Sender<Result<(), StoreError>>>,
    },
    /// A sentence and its recording to keep: the recording is stored first and recorded in the same batch, right
    /// before the sentence that refers to it (without it when storing failed).
    Sentence { record: Box<SentenceRecord>, audio: Bytes },
    /// Answered once everything handed over before is written (or failed).
    Barrier(oneshot::Sender<()>),
}

/// Where the recorder writes.
#[derive(Clone)]
pub(crate) struct Storage {
    pub log: Arc<dyn EventLog>,
    pub blobs: Arc<dyn BlobStore>,
}

/// Where to hand events (cheap to clone).
#[derive(Clone)]
pub(crate) struct Recorder {
    addr: Addr<RecordMsg>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").finish_non_exhaustive()
    }
}

fn new_events(events: &[Event]) -> Vec<NewEvent> {
    events.iter().filter_map(|e| e.to_new(None)).collect()
}

fn stopped() -> StoreError {
    StoreError::Io("the recorder has stopped".into())
}

impl Recorder {
    pub fn new(addr: Addr<RecordMsg>) -> Recorder {
        Recorder { addr }
    }

    /// Records `events` after everything handed over before, without waiting. A failure is logged (and a log that
    /// stopped writing shows on the System page).
    pub fn record(&self, events: Vec<Event>) {
        let events = new_events(&events);
        if !events.is_empty() && self.addr.send(RecordMsg::Events { events, ack: None }).is_err() {
            tracing::error!("events could not be recorded: the recorder has stopped");
        }
    }

    /// Records `events` and waits until they are on disk.
    pub async fn record_acked(&self, events: Vec<Event>) -> Result<(), StoreError> {
        let events = new_events(&events);
        if events.is_empty() {
            return Ok(());
        }
        let (tx, rx) = oneshot::channel();
        self.addr
            .send(RecordMsg::Events { events, ack: Some(tx) })
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }

    /// Records a sentence with the recording to keep (WAV), after everything handed over before, without waiting.
    pub fn sentence(&self, record: SentenceRecord, audio: Option<Bytes>) {
        let m = match audio {
            Some(audio) => RecordMsg::Sentence {
                record: Box::new(record),
                audio,
            },
            None => match Event::Sentence(Box::new(record)).to_new(None) {
                Some(e) => RecordMsg::Events {
                    events: vec![e],
                    ack: None,
                },
                None => return,
            },
        };
        if self.addr.send(m).is_err() {
            tracing::error!("a sentence could not be recorded: the recorder has stopped");
        }
    }

    /// Waits until everything handed over so far is written (or failed).
    pub async fn barrier(&self) {
        let (tx, rx) = oneshot::channel();
        if self.addr.send(RecordMsg::Barrier(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

/// The recorder actor.
pub(crate) struct Writer;

impl Supervised for Writer {
    type Ctx = Storage;
    type Msg = RecordMsg;
    const NAME: &'static str = "recorder";
    /// What it was writing when it crashed is lost (and logged); what waits in its mailbox is written after the
    /// restart, still in order.
    const POLICY: Policy = Policy::Restart;

    async fn start(_: &Storage) -> Result<Self, ActorError> {
        Ok(Writer)
    }

    /// Ends when every address is gone or at shutdown, after writing everything handed over (the log is drained).
    async fn run(self, store: Storage, mb: &mut Mailbox<RecordMsg>, life: Life) -> Result<(), ActorError> {
        loop {
            let first = tokio::select! {
                biased;
                m = mb.recv() => match m {
                    Some(m) => m,
                    None => return Ok(()),
                },
                () = life.cancel.cancelled() => match mb.try_recv() {
                    Some(m) => m,
                    None => return Ok(()),
                },
            };
            let mut batch = Vec::new();
            let mut acks = Vec::new();
            let mut barriers = Vec::new();
            let mut next = Some(first);
            while let Some(m) = next {
                match m {
                    RecordMsg::Events { events, ack } => {
                        batch.extend(events);
                        acks.extend(ack);
                    }
                    RecordMsg::Sentence { mut record, audio } => {
                        match store.blobs.put(audio).await {
                            Ok(info) => {
                                if info.new {
                                    batch.extend(
                                        Event::BlobAdded(BlobAdded {
                                            hash: info.hash,
                                            size: info.size,
                                            media_type: "audio/wav".into(),
                                            role: BlobRole::Recording,
                                        })
                                        .to_new(None),
                                    );
                                }
                                record.audio = Some(info.hash);
                            }
                            Err(e) => tracing::error!(error = %e, "a recording could not be kept"),
                        }
                        batch.extend(Event::Sentence(record).to_new(None));
                    }
                    RecordMsg::Barrier(tx) => barriers.push(tx),
                }
                next = mb.try_recv();
            }
            let result = if batch.is_empty() {
                Ok(())
            } else {
                store.log.append(batch).await.map(|_| ())
            };
            if let Err(e) = &result {
                tracing::error!(error = %e, "events could not be recorded");
            }
            for ack in acks {
                let _ = ack.send(result.clone());
            }
            for b in barriers {
                let _ = b.send(());
            }
        }
    }
}
