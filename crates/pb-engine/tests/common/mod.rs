//! The bot on stand-ins: the fake Fluxer through the real client, the in-process voice transport, the real store in a
//! temporary directory, and models that hear tones (`pb_testkit::models`). Everything runs in real time, without model
//! weights, a LiveKit server or a browser; a scenario takes a few seconds. Only the engine's public API is used.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test support

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use pb_domain::{BlobHash, ChannelId, ClfLang, GuildId, Scope, UserId};
use pb_engine::{Deps, Engine, SystemClock};
use pb_fluxer::{FluxerClient, GatewayConfig};
use pb_fluxer_fake::{FakeConfig, FakeFluxer, Sent};
use pb_infer::{Inference, Models};
use pb_models_api::TtsEngine;
use pb_settings::{Change, SettingKey, SettingsTree};
use pb_store::{FsBlobStore, FsSecretsFile, FsSettingsFiles, JsonlLog, TursoIndex};
use pb_store_api::{
    ActionRecord, BlobInfo, BlobStore, Event, EventLog, EventRef, NewEvent, PlayRecord, Secrets, SecretsFile,
    SentenceRecord, SettingsFiles, StoreError, StoredEvent, VerifyReport, WriterHealth,
};
use pb_testkit::memvoice::{MemVoice, Mic};
use pb_testkit::models::{BeepTts, Gate, ToneClassifier, ToneVad, tone};
use pb_voice_api::LISTEN_RATE;
use secrecy::SecretString;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::time::Instant;

pub const G: u64 = 111_111;
pub const VOICE: u64 = 222_222;
pub const TEXT: u64 = 333_333;
pub const ALICE: u64 = 444_444;
pub const OWNER: u64 = 1002;
/// View Channel, Send Messages, Attach Files, Connect, Speak.
pub const EVERYONE: u64 = (1 << 10) | (1 << 11) | (1 << 15) | (1 << 20) | (1 << 21);

/// One sentence: a second of a tone at `freq`, then a pause long enough to end it.
pub fn sentence(freq: f32) -> Vec<i16> {
    let mut pcm = tone(freq, 1.0, LISTEN_RATE, 0.3);
    pcm.resize(pcm.len() + LISTEN_RATE as usize * 4 / 5, 0);
    pcm
}

/// Waits until `f` holds, or fails the test after `secs` seconds.
pub async fn wait<F: Fn() -> bool>(what: &str, secs: u64, f: F) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the bot hears speech again after it spoke: for a second after its playback the microphone could still
/// carry its echo, and a sentence reaches back 0.3 s before it starts.
pub async fn echo_fades() {
    tokio::time::sleep(Duration::from_millis(1400)).await;
}

/// A setting changed before the start (as the owner).
pub fn set(tree: &mut SettingsTree, scope: Scope, key: SettingKey, value: serde_json::Value) -> Vec<Change> {
    tree.set(scope, key, value, true).unwrap().into_iter().collect()
}

/// The community's settings layer.
pub fn community() -> Scope {
    Scope::Server { guild: GuildId(G) }
}

/// Settings edits made before the start.
pub type Edit = Box<dyn FnOnce(&mut SettingsTree) -> Vec<Change>>;

/// How a scenario starts. Alice is tracked in community G; its mod log is TEXT.
pub struct Setup {
    pub settings: Edit,
    /// The bot may not speak in the voice channel.
    pub deny_speak: bool,
    /// Every classification waits at this gate.
    pub gate: Option<Gate>,
    /// The language the classifier hears.
    pub heard: ClfLang,
    /// The data of an earlier run (a restart), instead of a new directory.
    pub dir: Option<tempfile::TempDir>,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            settings: Box::new(|_| Vec::new()),
            deny_speak: false,
            gate: None,
            heard: ClfLang::En,
            dir: None,
        }
    }
}

/// How much longer every write to the event log or the blob store takes (none at first): a slow or busy disk.
#[derive(Debug, Clone, Default)]
pub struct DiskDelay(Arc<AtomicU64>);

impl DiskDelay {
    pub fn set(&self, delay: Duration) {
        self.0
            .store(u64::try_from(delay.as_millis()).unwrap(), Ordering::SeqCst);
    }

    async fn wait(&self) {
        let ms = self.0.load(Ordering::SeqCst);
        if ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }
}

/// The event log, slowed down by a [`DiskDelay`].
struct SlowLog {
    inner: JsonlLog,
    delay: DiskDelay,
}

#[async_trait]
impl EventLog for SlowLog {
    async fn append(&self, events: Vec<NewEvent>) -> Result<Vec<EventRef>, StoreError> {
        self.delay.wait().await;
        self.inner.append(events).await
    }

    fn head(&self) -> Option<EventRef> {
        self.inner.head()
    }

    fn health(&self) -> WriterHealth {
        self.inner.health()
    }

    async fn retry(&self) -> Result<(), StoreError> {
        self.inner.retry().await
    }

    fn scan(&self, from_seq: u64) -> BoxStream<'_, Result<StoredEvent, StoreError>> {
        self.inner.scan(from_seq)
    }

    async fn verify(&self) -> Result<VerifyReport, StoreError> {
        self.inner.verify().await
    }

    fn follow(&self) -> broadcast::Receiver<Arc<[StoredEvent]>> {
        self.inner.follow()
    }

    fn size(&self) -> u64 {
        self.inner.size()
    }
}

/// The blob store, slowed down by a [`DiskDelay`].
struct SlowBlobs {
    inner: FsBlobStore,
    delay: DiskDelay,
}

#[async_trait]
impl BlobStore for SlowBlobs {
    async fn put(&self, bytes: Bytes) -> Result<BlobInfo, StoreError> {
        self.delay.wait().await;
        self.inner.put(bytes).await
    }

    async fn put_file(&self, staged: &Path) -> Result<BlobInfo, StoreError> {
        self.delay.wait().await;
        self.inner.put_file(staged).await
    }

    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>, StoreError> {
        self.inner.get(hash).await
    }

    async fn path(&self, hash: &BlobHash) -> Option<PathBuf> {
        self.inner.path(hash).await
    }

    async fn delete(&self, hash: &BlobHash) -> Result<bool, StoreError> {
        self.inner.delete(hash).await
    }

    fn staging_dir(&self) -> &Path {
        self.inner.staging_dir()
    }

    fn size(&self) -> u64 {
        self.inner.size()
    }
}

pub struct Rig {
    pub fake: FakeFluxer,
    pub voice: MemVoice,
    pub engine: Engine,
    pub inference: Inference,
    /// What text-to-speech was asked to say.
    pub tts: BeepTts,
    /// How many clips the classifier was given.
    pub classified: Arc<AtomicUsize>,
    pub log: Arc<dyn EventLog>,
    pub index: Arc<TursoIndex>,
    pub disk: DiskDelay,
    pub dir: tempfile::TempDir,
    /// Alice's voice connection in the fake (once she joined).
    alice_conn: Mutex<Option<String>>,
}

impl Rig {
    pub async fn start(setup: Setup) -> Rig {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("warn,pb_engine=info")
            .with_test_writer()
            .try_init();
        let fake = FakeFluxer::start(FakeConfig::default()).await;
        fake.add_user(ALICE, "alice", Some("Alice"));
        fake.add_guild(G, "Alpha", OWNER, EVERYONE);
        fake.add_channel(G, VOICE, "voice", 2);
        fake.add_channel(G, TEXT, "mod-log", 0);
        fake.add_member(G, ALICE, &[]);
        fake.add_member(G, OWNER, &[]);
        let voice = MemVoice::default();
        if setup.deny_speak {
            voice.deny_speak(GuildId(G), ChannelId(VOICE));
        }

        let dir = setup.dir.unwrap_or_else(|| tempfile::tempdir().unwrap());
        let d = dir.path();
        let disk = DiskDelay::default();
        let log: Arc<dyn EventLog> = Arc::new(SlowLog {
            inner: JsonlLog::open(&d.join("log")).unwrap().0,
            delay: disk.clone(),
        });
        // A new index every start: the one of an earlier start in this process stays open (it follows the log until
        // the process ends). It is built from the log.
        static STARTS: AtomicUsize = AtomicUsize::new(0);
        let n = STARTS.fetch_add(1, Ordering::SeqCst);
        let index = Arc::new(
            TursoIndex::open(&d.join(format!("index/{n}.db")), log.clone())
                .await
                .unwrap(),
        );
        let blobs = Arc::new(SlowBlobs {
            inner: FsBlobStore::open(&d.join("blobs"), &d.join("tmp")).await.unwrap(),
            delay: disk.clone(),
        });
        let settings_files = Arc::new(FsSettingsFiles::new(&d.join("settings")));
        let secrets = Arc::new(FsSecretsFile::new(d));
        secrets
            .save(&Secrets {
                bot_token: Some(SecretString::from(fake.config().token.clone())),
                ..Secrets::default()
            })
            .await
            .unwrap();
        let (mut tree, _) = settings_files.load().await.unwrap();
        let mut changes = set(&mut tree, Scope::Global, SettingKey::Instance, json!(fake.url()));
        changes.extend(set(&mut tree, Scope::Global, SettingKey::JoinSettle, json!(0.0)));
        changes.extend(set(
            &mut tree,
            community(),
            SettingKey::ModlogChannel,
            json!(TEXT.to_string()),
        ));
        changes.extend(tree.track(GuildId(G), UserId(ALICE), None, jiff::Timestamp::now()));
        changes.extend((setup.settings)(&mut tree));
        settings_files.write(&changes).await.unwrap();

        let mut classifier = ToneClassifier::default().language(setup.heard);
        if let Some(gate) = setup.gate {
            classifier = classifier.gated(gate);
        }
        let classified = classifier.calls();
        let tts = BeepTts::default();
        let engine_tts = tts.clone();
        let inference = Inference::start(Models {
            vad: Box::new(ToneVad::default()),
            classifier: Box::new(classifier),
            tts: vec![Box::new(
                move |_| Ok(Box::new(engine_tts.clone()) as Box<dyn TtsEngine>),
            )],
            tts_threads: 1,
        })
        .unwrap();
        let deps = Deps {
            fluxer: Arc::new(FluxerClient {
                gateway: GatewayConfig {
                    backoff_base: Duration::from_millis(200),
                    ..GatewayConfig::default()
                },
            }),
            voice: Arc::new(voice.clone()),
            inference: inference.clone(),
            log: log.clone(),
            index: index.clone(),
            blobs,
            settings_files,
            secrets,
            hub: pb_live::Hub::new(),
            clock: Arc::new(SystemClock::default()),
            version: "test".into(),
            shipped_clips: Vec::new(),
            disk_free: Arc::new(|| None),
        };
        let engine = Engine::start(deps, tree).await.unwrap();
        Rig {
            fake,
            voice,
            engine,
            inference,
            tts,
            classified,
            log,
            index,
            disk,
            dir,
            alice_conn: Mutex::new(None),
        }
    }

    /// Alice joins the voice channel with her microphone on; returns once the bot is in the call and receives her.
    pub async fn alice_joins(&self) -> Mic {
        let conn = self.fake.voice_join(G, VOICE, ALICE);
        *self.alice_conn.lock().unwrap() = Some(conn.clone());
        let mic = self.voice.join(GuildId(G), ChannelId(VOICE), UserId(ALICE), &conn);
        wait("the bot joins and confirms", 10, || {
            self.fake.bot_connections().iter().any(|c| !c.3)
        })
        .await;
        wait("the bot listens to Alice", 10, || mic.heard()).await;
        mic
    }

    /// Alice's voice connection id in the fake.
    pub fn alice_connection(&self) -> String {
        self.alice_conn.lock().unwrap().clone().expect("Alice joined")
    }

    /// Every event so far, in log order.
    pub async fn events(&self) -> Vec<Event> {
        let all: Vec<_> = self.log.scan(1).map(|e| e.unwrap()).collect().await;
        all.iter().filter_map(|e| Event::from_stored(e).ok()).collect()
    }

    /// Waits until an event matches; returns the first match.
    pub async fn wait_event<F: Fn(&Event) -> bool>(&self, what: &str, secs: u64, f: F) -> Event {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let events = self.events().await;
            if let Some(e) = events.iter().find(|e| f(e)) {
                return e.clone();
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for: {what}; the log: {events:#?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits (up to 10 s) until a sentence record matches; returns the first match.
    pub async fn wait_sentence<F: Fn(&SentenceRecord) -> bool>(&self, what: &str, f: F) -> SentenceRecord {
        match self
            .wait_event(what, 10, |e| matches!(e, Event::Sentence(s) if f(s)))
            .await
        {
            Event::Sentence(s) => *s,
            _ => unreachable!(),
        }
    }

    /// Waits (up to 10 s) until a playback record matches; returns the first match.
    pub async fn wait_played<F: Fn(&PlayRecord) -> bool>(&self, what: &str, f: F) -> PlayRecord {
        match self
            .wait_event(what, 10, |e| matches!(e, Event::Played(p) if f(p)))
            .await
        {
            Event::Played(p) => *p,
            _ => unreachable!(),
        }
    }

    /// Waits (up to 10 s) until an action record matches; returns the first match.
    pub async fn wait_action<F: Fn(&ActionRecord) -> bool>(&self, what: &str, f: F) -> ActionRecord {
        match self
            .wait_event(what, 10, |e| matches!(e, Event::Action(a) if f(a)))
            .await
        {
            Event::Action(a) => *a,
            _ => unreachable!(),
        }
    }

    /// Every playback record so far.
    pub async fn played(&self) -> Vec<PlayRecord> {
        self.events()
            .await
            .into_iter()
            .filter_map(|e| match e {
                Event::Played(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    /// The bot's answers to a chat message.
    pub fn replies_to(&self, message: u64) -> Vec<Sent> {
        let id = message.to_string();
        self.fake
            .sent()
            .into_iter()
            .filter(|m| m.payload["message_reference"]["message_id"].as_str() == Some(id.as_str()))
            .collect()
    }

    /// Whether the bot reacted to a chat message with `emoji`.
    pub fn reacted(&self, message: u64, emoji: &str) -> bool {
        self.fake
            .reactions()
            .iter()
            .any(|(_, m, e)| *m == message && e == emoji)
    }

    /// Stops the bot and its models and keeps only its data (for a restart).
    pub async fn stop_keeping_data(self) -> tempfile::TempDir {
        self.stop().await;
        self.dir
    }

    /// Stops the bot and its models.
    pub async fn stop(&self) {
        self.engine.shutdown().await;
        let inference = self.inference.clone();
        tokio::task::spawn_blocking(move || inference.shutdown()).await.unwrap();
    }
}
