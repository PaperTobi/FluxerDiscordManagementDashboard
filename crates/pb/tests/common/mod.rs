//! A running bot against a fake Fluxer and a real LiveKit server (or the in-process voice transport), with the real
//! models.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test support

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use pb_domain::{ChannelId, GuildId, Scope, UserId};
use pb_engine::{Deps, Engine, SystemClock};
use pb_fluxer::{FluxerClient, GatewayConfig};
use pb_fluxer_fake::{FakeConfig, FakeFluxer};
use pb_infer::{Inference, Models};
use pb_models_api::TtsEngine;
use pb_settings::{Change, SettingKey, SettingsTree};
use pb_store::{FsBlobStore, FsSecretsFile, FsSettingsFiles, JsonlLog, TursoIndex};
use pb_store_api::{Event, EventLog, Secrets, SecretsFile, SettingsFiles};
use pb_testkit::lk::{LiveKitServer, Person};
use pb_testkit::memvoice::{MemVoice, Mic};
use secrecy::SecretString;

pub const G: u64 = 111_111;
pub const VOICE: u64 = 222_222;
pub const TEXT: u64 = 333_333;
pub const ALICE: u64 = 444_444;
pub const OWNER: u64 = 1002;
/// View Channel, Send Messages, Attach Files, Connect, Speak.
pub const EVERYONE: u64 = (1 << 10) | (1 << 11) | (1 << 15) | (1 << 20) | (1 << 21);

pub fn weights() -> PathBuf {
    pb_testkit::weights()
}

pub fn inference() -> Inference {
    let w = weights();
    let voices = w.join("voices");
    Inference::start(Models {
        vad: Box::new(pb_vad_silero::SileroVad::load(&w.join("silero-vad")).unwrap()),
        classifier: Box::new(
            pb_classifier_roblox::RobloxClassifier::load_cpu(
                &w.join("roblox-voice-safety-v3"),
                NonZeroUsize::new(4).unwrap(),
            )
            .unwrap(),
        ),
        tts: Some(Box::new(move |threads| {
            pb_tts_piper::PiperEngine::new(
                std::path::Path::new(pb_espeak::BUILD_DATA_DIR),
                std::slice::from_ref(&voices),
                threads,
            )
            .map(|e| Box::new(e) as Box<dyn TtsEngine>)
        })),
        tts_threads: 2,
    })
    .unwrap()
}

pub fn speech48(name: &str) -> Vec<i16> {
    let (pcm, rate) = pb_testkit::audio::read_wav(&pb_testkit::fixture(name)).unwrap();
    pb_testkit::audio::resample(&pcm, rate, 48_000).unwrap()
}

pub async fn wait<F: Fn() -> bool>(what: &str, secs: u64, f: F) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(std::time::Instant::now() < end, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Settings edits made before the start.
pub type Edit = Box<dyn FnOnce(&mut SettingsTree) -> Vec<Change>>;

/// How a scenario starts.
pub struct Setup {
    /// The bot may speak in voice.
    pub can_speak: bool,
    /// Settings edits before the start.
    pub settings: Edit,
    /// Voice through this in-process transport instead of a LiveKit server.
    pub memory_voice: Option<MemVoice>,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            can_speak: true,
            settings: Box::new(|_| Vec::new()),
            memory_voice: None,
        }
    }
}

pub struct Rig {
    /// The LiveKit server (none with the in-process transport).
    pub lk: Option<Arc<LiveKitServer>>,
    pub inference: Inference,
    pub fake: FakeFluxer,
    pub engine: Engine,
    pub log: Arc<dyn EventLog>,
    pub index: Arc<TursoIndex>,
    pub blobs: Arc<FsBlobStore>,
    pub dir: tempfile::TempDir,
    /// Alice's voice connection in the fake (once she joined).
    alice_conn: std::sync::Mutex<Option<String>>,
}

impl Rig {
    pub async fn start(setup: Setup) -> Rig {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("warn,pb_engine=info")
            .with_test_writer()
            .try_init();
        let lk = setup
            .memory_voice
            .is_none()
            .then(|| Arc::new(LiveKitServer::start().unwrap()));
        let mut fake_cfg = FakeConfig::default();
        if let Some(lk) = &lk {
            let url = lk.url();
            let can_speak = setup.can_speak;
            fake_cfg.grant = Arc::new(move |g, c, bot, conn| {
                (
                    url.clone(),
                    pb_testkit::lk::mint(
                        &format!("user_{bot}_{conn}"),
                        &format!("guild_{g}_channel_{c}"),
                        can_speak,
                    ),
                )
            });
        }
        let fake = FakeFluxer::start(fake_cfg).await;
        fake.add_user(ALICE, "alice", Some("Alice"));
        fake.add_guild(G, "Alpha", OWNER, EVERYONE);
        fake.add_channel(G, VOICE, "voice", 2);
        fake.add_channel(G, TEXT, "mod-log", 0);
        fake.add_member(G, ALICE, &[]);
        fake.add_member(G, OWNER, &[]);

        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&d.join("log")).unwrap().0);
        let index = Arc::new(TursoIndex::open(&d.join("index/index.db"), log.clone()).await.unwrap());
        let blobs = Arc::new(FsBlobStore::open(&d.join("blobs"), &d.join("tmp")).await.unwrap());
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
        let mut changes = Vec::new();
        changes.extend(
            tree.set(Scope::Global, SettingKey::Instance, serde_json::json!(fake.url()), true)
                .unwrap(),
        );
        changes.extend(
            tree.set(Scope::Global, SettingKey::JoinSettle, serde_json::json!(0.0), true)
                .unwrap(),
        );
        changes.extend(
            tree.set(
                Scope::Server { guild: GuildId(G) },
                SettingKey::ModlogChannel,
                serde_json::json!(TEXT.to_string()),
                true,
            )
            .unwrap(),
        );
        changes.extend(tree.track(GuildId(G), UserId(ALICE), None, jiff::Timestamp::now()));
        changes.extend((setup.settings)(&mut tree));
        settings_files.write(&changes).await.unwrap();

        let inference = inference();
        let deps = Deps {
            fluxer: Arc::new(FluxerClient {
                gateway: GatewayConfig {
                    backoff_base: Duration::from_millis(100),
                    ..GatewayConfig::default()
                },
            }),
            voice: match setup.memory_voice {
                Some(m) => Arc::new(m),
                None => Arc::new(pb_voice_livekit::LiveKitTransport),
            },
            inference: inference.clone(),
            log: log.clone(),
            index: index.clone(),
            blobs: blobs.clone(),
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
            lk,
            inference,
            fake,
            engine,
            log,
            index,
            blobs,
            dir,
            alice_conn: std::sync::Mutex::new(None),
        }
    }

    /// Alice joins voice and publishes her microphone; returns once the bot listens to her.
    pub async fn alice_joins(&self) -> (Person, pb_testkit::lk::Mic) {
        let conn = self.fake.voice_join(G, VOICE, ALICE);
        *self.alice_conn.lock().unwrap() = Some(conn.clone());
        let bot = self.fake.config().bot_id;
        let room = format!("guild_{G}_channel_{VOICE}");
        let alice = Person::join(
            self.lk.as_ref().expect("a LiveKit rig"),
            &format!("user_{ALICE}_{conn}"),
            &room,
            &format!("user_{bot}_"),
        )
        .await
        .unwrap();
        let mic = alice.publish_mic().await.unwrap();
        wait("the bot joins and confirms", 20, || {
            self.fake.bot_connections().iter().any(|c| !c.3)
        })
        .await;
        wait("the bot listens to Alice", 20, || alice.times_subscribed_to() >= 1).await;
        (alice, mic)
    }

    /// Alice joins voice through the in-process transport; returns once the bot listens to her.
    pub async fn alice_joins_memory(&self, voice: &MemVoice) -> Mic {
        let conn = self.fake.voice_join(G, VOICE, ALICE);
        *self.alice_conn.lock().unwrap() = Some(conn.clone());
        let mic = voice.join(GuildId(G), ChannelId(VOICE), UserId(ALICE), &conn);
        wait("the bot joins and confirms", 20, || {
            self.fake.bot_connections().iter().any(|c| !c.3)
        })
        .await;
        wait("the bot listens to Alice", 20, || mic.heard()).await;
        mic
    }

    /// Alice's voice connection id in the fake.
    pub fn alice_connection(&self) -> String {
        self.alice_conn.lock().unwrap().clone().expect("Alice joined")
    }

    /// Every event so far.
    pub async fn events(&self) -> Vec<Event> {
        let all: Vec<_> = self.log.scan(1).map(|e| e.unwrap()).collect().await;
        all.iter().filter_map(|e| Event::from_stored(e).ok()).collect()
    }

    /// Waits until an event matches.
    pub async fn wait_event<F: Fn(&Event) -> bool>(&self, what: &str, secs: u64, f: F) -> Event {
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(e) = self.events().await.into_iter().find(|e| f(e)) {
                return e;
            }
            assert!(std::time::Instant::now() < end, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// How many loud samples Alice heard from the bot.
    pub fn alice_heard(&self, alice: &Person) -> usize {
        let bot = self.fake.config().bot_id;
        self.fake.bot_connections().first().map_or(0, |(c, _, _, _)| {
            alice
                .heard_from(&format!("user_{bot}_{c}"))
                .iter()
                .filter(|s| s.unsigned_abs() > 500)
                .count()
        })
    }

    pub async fn stop(self, alice: Option<Person>) {
        self.engine.shutdown().await;
        if let Some(a) = alice {
            let _ = a.leave().await;
        }
        // Frees the models (each rig loads its own).
        let inference = self.inference.clone();
        let _ = tokio::task::spawn_blocking(move || inference.shutdown()).await;
    }
}
