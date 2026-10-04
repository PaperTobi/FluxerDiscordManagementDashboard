//! Version 1.

mod queue;

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc as smpsc};
use std::thread::JoinHandle;
use std::time::Instant;

use pb_models_api::{
    Classifier, ClassifierInfo, FRAME, ModelError, RawScores, SpeakOpts, TtsEngine, TtsError, VadInfo, VadModel,
    VadState, VoiceInfo, energy_gate,
};
use tokio::sync::oneshot;

pub use queue::{Queue, QueueStats};

/// The classifier's longest window (30 s at 16 kHz) and the hop between windows of longer speech.
pub const WINDOW: usize = 480_000;
pub const HOP: usize = 400_000;

/// Who waits for a classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Imported or re-scored history.
    Import = 0,
    /// A clip's self-check, a re-check asked for in the UI.
    Check = 1,
    /// Someone just said it.
    Live = 2,
}

/// How a classification is scheduled: its priority, when its answer stops being useful (it is still done after that,
/// behind the jobs that can still be on time), and its flow (one person's jobs are done in the order they came).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    pub priority: Priority,
    pub deadline: Option<Instant>,
    pub flow: Option<u64>,
}

impl From<Priority> for Job {
    fn from(priority: Priority) -> Job {
        Job {
            priority,
            deadline: None,
            flow: None,
        }
    }
}

/// Who waits for speech.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpeakPriority {
    /// Rendering ahead of time (the next likely warnings).
    Prerender = 0,
    /// A preview in the web UI.
    Preview = 1,
    /// A warning that plays now.
    Live = 2,
}

/// Runs a text-to-speech engine with the given number of threads (called again to apply a new count or newly
/// installed voices).
pub type TtsFactory = Box<dyn Fn(usize) -> Result<Box<dyn TtsEngine>, TtsError> + Send>;

/// The models, each moved onto its thread.
pub struct Models {
    pub vad: Box<dyn VadModel>,
    pub classifier: Box<dyn Classifier>,
    /// The speech models (Piper, and others), each on its own thread.
    pub tts: Vec<TtsFactory>,
    pub tts_threads: usize,
}

impl std::fmt::Debug for Models {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Models")
            .field("vad", self.vad.info())
            .field("classifier", self.classifier.info())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InferError {
    #[error("the inference threads have stopped")]
    Stopped,
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error(transparent)]
    Tts(#[from] TtsError),
    #[error("text-to-speech is not available")]
    NoTts,
    /// The model failed inside (a bug in it or its library); the thread goes on with the next job.
    #[error("the model failed: {0}")]
    Panicked(String),
}

/// A thread that owns a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worker {
    Vad,
    Classifier,
    Speech,
}

impl Worker {
    pub fn name(self) -> &'static str {
        match self {
            Worker::Vad => "voice activity",
            Worker::Classifier => "classifier",
            Worker::Speech => "text-to-speech",
        }
    }
}

/// Runs `f`; a panic inside it becomes an error (the panic message) instead of ending the model's thread.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".to_owned())
    })
}

/// A classified sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub raw: RawScores,
    /// Model time.
    pub infer_ms: u32,
    /// Time in the queue.
    pub queued_ms: u32,
    /// How many 30 s windows (1 for anything up to 30 s).
    pub windows: u32,
}

/// Speech at 48 kHz.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech48 {
    pub samples: Vec<i16>,
    pub unknown_phonemes: Vec<String>,
    pub infer_ms: u32,
}

/// What the System page shows.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct InferStatus {
    pub vad_model: String,
    pub vad_streams: u64,
    pub vad_frames: u64,
    /// Frames where the model gave no usable number and the energy gate answered instead.
    pub vad_fallbacks: u64,
    pub classifier_model: String,
    pub classifier_device: String,
    pub classify: QueueStats,
    pub speak: QueueStats,
    pub voices: usize,
}

fn ms(d: std::time::Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

// ------------------------------------------------------------------------------------------------ VAD

enum VadMsg {
    Open(u64),
    Step {
        id: u64,
        frames: Vec<[f32; FRAME]>,
        reply: oneshot::Sender<Vec<f32>>,
    },
    Reset(u64),
    Close(u64),
    Stop,
}

struct VadStreamState {
    state: VadState,
    /// Energy-gate floor, for frames the model cannot answer.
    floor: f32,
    pending: VecDeque<[f32; FRAME]>,
    out: Vec<f32>,
    reply: Option<oneshot::Sender<Vec<f32>>>,
}

struct VadCounters {
    streams: AtomicU64,
    frames: AtomicU64,
    fallbacks: AtomicU64,
}

fn vad_thread(mut model: Box<dyn VadModel>, rx: smpsc::Receiver<VadMsg>, counters: Arc<VadCounters>) {
    let mut streams: HashMap<u64, VadStreamState> = HashMap::new();
    let handle = |msg: VadMsg, streams: &mut HashMap<u64, VadStreamState>, model: &dyn VadModel| -> bool {
        match msg {
            VadMsg::Open(id) => {
                streams.insert(
                    id,
                    VadStreamState {
                        state: model.new_state(),
                        floor: f32::NAN,
                        pending: VecDeque::new(),
                        out: Vec::new(),
                        reply: None,
                    },
                );
                counters.streams.store(streams.len() as u64, Ordering::Relaxed);
            }
            VadMsg::Step { id, frames, reply } => match streams.get_mut(&id) {
                Some(s) if !frames.is_empty() => {
                    s.pending.extend(frames);
                    s.reply = Some(reply);
                }
                _ => {
                    let _ = reply.send(Vec::new());
                }
            },
            VadMsg::Reset(id) => {
                if let Some(s) = streams.get_mut(&id) {
                    s.state = model.new_state();
                    s.floor = f32::NAN;
                }
            }
            VadMsg::Close(id) => {
                streams.remove(&id);
                counters.streams.store(streams.len() as u64, Ordering::Relaxed);
            }
            VadMsg::Stop => return false,
        }
        true
    };
    loop {
        let Ok(first) = rx.recv() else { return };
        if !handle(first, &mut streams, &*model) {
            return;
        }
        while let Ok(more) = rx.try_recv() {
            if !handle(more, &mut streams, &*model) {
                return;
            }
        }
        // One frame of every waiting stream per batched step, until all are done.
        loop {
            let mut ids: Vec<u64> = streams
                .iter()
                .filter(|(_, s)| !s.pending.is_empty())
                .map(|(id, _)| *id)
                .collect();
            if ids.is_empty() {
                break;
            }
            ids.sort_unstable();
            let frames: Vec<[f32; FRAME]> = ids
                .iter()
                .filter_map(|id| streams.get_mut(id)?.pending.pop_front())
                .collect();
            let mut taken: Vec<(u64, VadStreamState)> = ids
                .iter()
                .filter_map(|id| streams.remove(id).map(|s| (*id, s)))
                .collect();
            // A failing model answers nothing: every stream falls back to the energy gate below.
            let probs = guarded(|| {
                let mut states: Vec<&mut VadState> = taken.iter_mut().map(|(_, s)| &mut s.state).collect();
                model.step(&frames, &mut states)
            })
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "the voice activity model failed");
                Vec::new()
            });
            counters.frames.fetch_add(frames.len() as u64, Ordering::Relaxed);
            for (i, (id, mut s)) in taken.into_iter().enumerate() {
                let energy = energy_gate(&mut s.floor, &frames[i]);
                let p = match probs.get(i) {
                    Some(p) if p.is_finite() => p.clamp(0.0, 1.0),
                    _ => {
                        // The model's state is unusable for this stream: answer with the energy gate, start over.
                        counters.fallbacks.fetch_add(1, Ordering::Relaxed);
                        s.state = model.new_state();
                        energy
                    }
                };
                s.out.push(p);
                if s.pending.is_empty()
                    && let Some(reply) = s.reply.take()
                {
                    let _ = reply.send(std::mem::take(&mut s.out));
                }
                streams.insert(id, s);
            }
        }
    }
}

/// One person's voice-activity stream.
#[derive(Debug)]
pub struct VadStream {
    id: u64,
    tx: smpsc::Sender<VadMsg>,
}

impl VadStream {
    /// The speech probability of each frame, in order (the stream's state carries over between calls).
    pub async fn step(&mut self, frames: Vec<[f32; FRAME]>) -> Result<Vec<f32>, InferError> {
        if frames.is_empty() {
            return Ok(Vec::new());
        }
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(VadMsg::Step {
                id: self.id,
                frames,
                reply,
            })
            .map_err(|_| InferError::Stopped)?;
        rx.await.map_err(|_| InferError::Stopped)
    }

    /// Forgets the state (after a long pause such as a mute).
    pub fn reset(&self) {
        let _ = self.tx.send(VadMsg::Reset(self.id));
    }
}

impl Drop for VadStream {
    fn drop(&mut self) {
        let _ = self.tx.send(VadMsg::Close(self.id));
    }
}

// ------------------------------------------------------------------------------------------------ classifier

enum ClfJob {
    Classify {
        pcm: Arc<[f32]>,
        reply: oneshot::Sender<Result<Scored, InferError>>,
    },
    Threads(NonZeroUsize),
}

fn classify_long(model: &mut dyn Classifier, pcm: &[f32]) -> Result<(RawScores, u32), ModelError> {
    if pcm.len() <= WINDOW {
        return model.classify(pcm).map(|r| (r, 1));
    }
    let wins = pb_segment::windows(pcm.len(), WINDOW, HOP);
    let mut labels = [0.0f32; 8];
    let mut languages = [0.0f32; 30];
    let mut weight = 0.0f32;
    for w in &wins {
        let r = model.classify(&pcm[w.start..w.start + w.len])?;
        for (a, b) in labels.iter_mut().zip(r.labels) {
            *a = a.max(b);
        }
        let wt = w.len as f32;
        for (a, b) in languages.iter_mut().zip(r.languages) {
            *a += b * wt;
        }
        weight += wt;
    }
    for a in &mut languages {
        *a /= weight.max(1.0);
    }
    Ok((
        RawScores { labels, languages },
        u32::try_from(wins.len()).unwrap_or(u32::MAX),
    ))
}

fn classifier_thread(mut model: Box<dyn Classifier>, queue: Arc<Queue<ClfJob>>) {
    while let Some((job, waited)) = queue.pop() {
        match job {
            ClfJob::Threads(n) => {
                if let Err(e) = model.set_threads(n) {
                    tracing::warn!(error = %e, "could not change the classifier's threads");
                }
            }
            ClfJob::Classify { pcm, reply } => {
                if reply.is_closed() {
                    continue; // nobody waits any more
                }
                let t = Instant::now();
                let result = guarded(|| classify_long(&mut *model, &pcm))
                    .map_err(InferError::Panicked)
                    .and_then(|r| r.map_err(InferError::from))
                    .map(|(raw, windows)| Scored {
                        raw,
                        infer_ms: ms(t.elapsed()),
                        queued_ms: ms(waited),
                        windows,
                    });
                let _ = reply.send(result);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ speech

enum TtsJob {
    Speak {
        voice: String,
        text: String,
        opts: SpeakOpts,
        reply: oneshot::Sender<Result<Speech48, InferError>>,
    },
    Reload {
        threads: usize,
        reply: oneshot::Sender<Result<Vec<VoiceInfo>, InferError>>,
    },
}

fn tts_thread(factory: TtsFactory, threads: usize, queue: Arc<Queue<TtsJob>>, voices: Arc<Mutex<Vec<VoiceInfo>>>) {
    // `voices` is this engine's own list.
    let set_voices = |list: Vec<VoiceInfo>| {
        if let Ok(mut v) = voices.lock() {
            *v = list;
        }
    };
    let mut engine = match factory(threads) {
        Ok(e) => {
            set_voices(e.voices());
            Some(e)
        }
        Err(e) => {
            tracing::error!(error = %e, "text-to-speech could not start");
            None
        }
    };
    while let Some((job, _)) = queue.pop() {
        match job {
            TtsJob::Reload { threads, reply } => {
                let result = factory(threads).map_err(InferError::from).map(|e| {
                    let list = e.voices();
                    set_voices(list.clone());
                    engine = Some(e);
                    list
                });
                let _ = reply.send(result);
            }
            TtsJob::Speak {
                voice,
                text,
                opts,
                reply,
            } => {
                if reply.is_closed() {
                    continue;
                }
                let t = Instant::now();
                let result = match engine.as_mut() {
                    None => Err(InferError::NoTts),
                    Some(e) => guarded(|| e.synthesize(&voice, &text, &opts))
                        .map_err(InferError::Panicked)
                        .and_then(|r| r.map_err(InferError::from))
                        .and_then(|s| {
                            let mut at48 = pb_audio::resample(&s.samples, s.sample_rate, pb_audio::PLAY_RATE)
                                .map_err(|e| InferError::Tts(TtsError::Failed(e.to_string())))?;
                            // Piper normalises to full scale; 1 dB of headroom keeps the resampled peaks unclipped.
                            pb_audio::limit_peak(&mut at48, -1.0);
                            Ok(Speech48 {
                                samples: pb_audio::to_i16(&at48),
                                unknown_phonemes: s.unknown_phonemes,
                                infer_ms: ms(t.elapsed()),
                            })
                        }),
                };
                let _ = reply.send(result);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ the handle

struct Inner {
    vad_tx: smpsc::Sender<VadMsg>,
    next_stream: AtomicU64,
    vad_counters: Arc<VadCounters>,
    vad_info: VadInfo,
    clf: Arc<Queue<ClfJob>>,
    clf_info: ClassifierInfo,
    tts: Vec<SpeechModel>,
    threads: Mutex<Vec<(Worker, JoinHandle<()>)>>,
}

/// A speech model's queue and its voices (kept by its thread).
struct SpeechModel {
    queue: Arc<Queue<TtsJob>>,
    voices: Arc<Mutex<Vec<VoiceInfo>>>,
}

impl SpeechModel {
    fn voices(&self) -> Vec<VoiceInfo> {
        self.voices.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

/// The way to the models.
#[derive(Clone)]
pub struct Inference {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Inference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inference").field("status", &self.status()).finish()
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name(name.into()).spawn(f)
}

impl Inference {
    /// Moves the models onto their threads.
    pub fn start(models: Models) -> std::io::Result<Inference> {
        let vad_info = models.vad.info().clone();
        let clf_info = models.classifier.info().clone();
        let (vad_tx, vad_rx) = smpsc::channel();
        let vad_counters = Arc::new(VadCounters {
            streams: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
        });
        let clf = Arc::new(Queue::default());
        let mut threads = Vec::new();
        let vad = models.vad;
        let counters = vad_counters.clone();
        threads.push((Worker::Vad, spawn("pb-vad", move || vad_thread(vad, vad_rx, counters))?));
        let classifier = models.classifier;
        let q = clf.clone();
        threads.push((
            Worker::Classifier,
            spawn("pb-classify", move || classifier_thread(classifier, q))?,
        ));
        let mut tts = Vec::new();
        for factory in models.tts {
            let q = Arc::new(Queue::default());
            let voices = Arc::new(Mutex::new(Vec::new()));
            let (q2, v2, n) = (q.clone(), voices.clone(), models.tts_threads);
            threads.push((Worker::Speech, spawn("pb-tts", move || tts_thread(factory, n, q2, v2))?));
            tts.push(SpeechModel { queue: q, voices });
        }
        Ok(Inference {
            inner: Arc::new(Inner {
                vad_tx,
                next_stream: AtomicU64::new(1),
                vad_counters,
                vad_info,
                clf,
                clf_info,
                tts,
                threads: Mutex::new(threads),
            }),
        })
    }

    /// A new voice-activity stream (one per microphone).
    pub fn vad_stream(&self) -> VadStream {
        let id = self.inner.next_stream.fetch_add(1, Ordering::Relaxed);
        let _ = self.inner.vad_tx.send(VadMsg::Open(id));
        VadStream {
            id,
            tx: self.inner.vad_tx.clone(),
        }
    }

    /// Scores 16 kHz speech (longer than 30 s in 30 s windows: the highest score of each type, languages weighted by
    /// window length).
    pub async fn classify(&self, pcm: Arc<[f32]>, job: impl Into<Job>) -> Result<Scored, InferError> {
        let job = job.into();
        let (reply, rx) = oneshot::channel();
        if !self.inner.clf.push_job(
            job.priority as u8,
            job.deadline,
            job.flow,
            ClfJob::Classify { pcm, reply },
        ) {
            return Err(InferError::Stopped);
        }
        rx.await.map_err(|_| InferError::Stopped)?
    }

    /// Speaks `text` with `voice`, at 48 kHz.
    pub async fn speak(
        &self,
        voice: &str,
        text: &str,
        opts: SpeakOpts,
        prio: SpeakPriority,
    ) -> Result<Speech48, InferError> {
        if self.inner.tts.is_empty() {
            return Err(InferError::NoTts);
        }
        // The engine that has the voice (the first one when none does: it says which voice is missing).
        let q = &self
            .inner
            .tts
            .iter()
            .find(|m| m.voices().iter().any(|v| v.named(voice)))
            .or_else(|| self.inner.tts.first())
            .ok_or(InferError::NoTts)?
            .queue;
        let id = self
            .voices()
            .into_iter()
            .find(|v| v.named(voice))
            .map_or_else(|| voice.to_owned(), |v| v.id);
        let (reply, rx) = oneshot::channel();
        if !q.push(
            prio as u8,
            TtsJob::Speak {
                voice: id,
                text: text.to_owned(),
                opts,
                reply,
            },
        ) {
            return Err(InferError::Stopped);
        }
        rx.await.map_err(|_| InferError::Stopped)?
    }

    /// The installed voices of every speech model.
    pub fn voices(&self) -> Vec<VoiceInfo> {
        self.inner.tts.iter().flat_map(SpeechModel::voices).collect()
    }

    /// Restarts every speech model with `threads` threads (and picks up newly installed voices).
    pub async fn reload_tts(&self, threads: usize) -> Result<Vec<VoiceInfo>, InferError> {
        if self.inner.tts.is_empty() {
            return Err(InferError::NoTts);
        }
        let mut all = Vec::new();
        for m in &self.inner.tts {
            let (reply, rx) = oneshot::channel();
            if !m.queue.push(u8::MAX, TtsJob::Reload { threads, reply }) {
                return Err(InferError::Stopped);
            }
            all.extend(rx.await.map_err(|_| InferError::Stopped)??);
        }
        Ok(all)
    }

    /// Changes the classifier's thread count (before the next job).
    pub fn set_classifier_threads(&self, n: NonZeroUsize) {
        self.inner.clf.push(u8::MAX, ClfJob::Threads(n));
    }

    pub fn classifier_info(&self) -> &ClassifierInfo {
        &self.inner.clf_info
    }

    pub fn status(&self) -> InferStatus {
        let i = &self.inner;
        InferStatus {
            vad_model: i.vad_info.model.clone(),
            vad_streams: i.vad_counters.streams.load(Ordering::Relaxed),
            vad_frames: i.vad_counters.frames.load(Ordering::Relaxed),
            vad_fallbacks: i.vad_counters.fallbacks.load(Ordering::Relaxed),
            classifier_model: i.clf_info.model.clone(),
            classifier_device: i.clf_info.device.clone(),
            classify: i.clf.stats(),
            speak: i
                .tts
                .iter()
                .map(|m| m.queue.stats())
                .fold(QueueStats::default(), QueueStats::merge),
            voices: self.voices().len(),
        }
    }

    /// A model thread that ended, or has been on one job for longer than any job takes (a hung GPU driver): the
    /// process should be restarted.
    pub fn stuck(&self) -> Option<Worker> {
        /// Far beyond the longest job (a ten-minute sentence is scored in about a minute on a CPU).
        const STUCK_MS: u64 = 10 * 60 * 1000;
        let ended = self
            .inner
            .threads
            .lock()
            .ok()
            .and_then(|t| t.iter().find(|(_, h)| h.is_finished()).map(|(w, _)| *w));
        if ended.is_some() {
            return ended;
        }
        let st = self.status();
        if st.classify.running_ms.is_some_and(|ms| ms > STUCK_MS) {
            Some(Worker::Classifier)
        } else if st.speak.running_ms.is_some_and(|ms| ms > STUCK_MS) {
            Some(Worker::Speech)
        } else {
            None
        }
    }

    /// Finishes the queued jobs and stops the threads.
    pub fn shutdown(&self) {
        let i = &self.inner;
        i.clf.close();
        for m in &i.tts {
            m.queue.close();
        }
        let _ = i.vad_tx.send(VadMsg::Stop);
        let handles: Vec<_> = i
            .threads
            .lock()
            .map(|mut t| std::mem::take(&mut *t))
            .unwrap_or_default();
        for (_, h) in handles {
            let _ = h.join();
        }
    }
}
