//! A running web server on a real engine against the fake Fluxer, with stand-in models and no voice.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test support

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pb_domain::{GuildId, Scope, UserId};
use pb_engine::{Connection, Deps, Engine, SystemClock};
use pb_fluxer_api::VoiceGrant;
use pb_fluxer_fake::{FakeConfig, FakeFluxer};
use pb_infer::{Inference, Models};
use pb_models_api::{Classifier, ClassifierInfo, ModelError, RawScores};
use pb_settings::SettingKey;
use pb_store::{FsBlobStore, FsSecretsFile, FsSessionsFile, FsSettingsFiles, JsonlLog, TursoIndex};
use pb_store_api::{EventLog, Secrets, SecretsFile, SettingsFiles, SetupState};
use pb_voice_api::{ConnectOpts, RoomEvent, TransportError, VoiceRoom, VoiceTransport};
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use secrecy::SecretString;
use tokio::sync::{mpsc, watch};

pub const G: u64 = 111_111;
pub const G2: u64 = 777_777;
/// Alpha's voice channel.
pub const LOUNGE: u64 = 222_222;
pub const OWNER: u64 = 1002;
/// Admin of Alpha through a role with Manage Community.
pub const ADA: u64 = 444_444;
/// A member of Alpha without rights.
pub const MAX: u64 = 555_555;
/// Owner of Beta (and not in Alpha).
pub const BEA: u64 = 666_666;
const MODS: u64 = 888_888;
const EVERYONE: u64 = (1 << 10) | (1 << 11) | (1 << 15) | (1 << 20) | (1 << 21);

/// Scores nothing.
struct Quiet(ClassifierInfo);

impl Classifier for Quiet {
    fn info(&self) -> &ClassifierInfo {
        &self.0
    }
    fn classify(&mut self, _: &[f32]) -> Result<RawScores, ModelError> {
        Ok(RawScores {
            labels: [0.0; 8],
            languages: [0.0; 30],
        })
    }
    fn set_threads(&mut self, _: NonZeroUsize) -> Result<(), ModelError> {
        Ok(())
    }
}

/// Nobody talks in these tests.
struct NoVoice;

#[async_trait]
impl VoiceTransport for NoVoice {
    async fn connect(
        &self,
        _: &VoiceGrant,
        _: ConnectOpts,
    ) -> Result<(Box<dyn VoiceRoom>, mpsc::UnboundedReceiver<RoomEvent>), TransportError> {
        Err(TransportError::Connect("no voice in these tests".into()))
    }
}

pub struct Web {
    pub fake: FakeFluxer,
    pub engine: Arc<Engine>,
    pub base: String,
    pub dir: tempfile::TempDir,
    pub http: reqwest::Client,
    pub log: Arc<dyn EventLog>,
    stop: watch::Sender<bool>,
    inference: Inference,
}

/// A response, read.
#[derive(Debug)]
pub struct Got {
    pub status: u16,
    pub location: Option<String>,
    pub csp: Option<String>,
    pub cookies: Vec<String>,
    pub body: String,
}

impl Got {
    /// The value of a cookie this response set.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.iter().find_map(|c| {
            let first = c.split(';').next()?;
            let (k, v) = first.split_once('=')?;
            (k == name && !v.is_empty()).then(|| v.to_owned())
        })
    }
}

impl Web {
    /// `ready`: setup is finished (token, client secret, owner); otherwise a fresh data directory.
    pub async fn start(ready: bool) -> Web {
        Web::start_with(ready, false).await
    }

    /// The same, over HTTPS with the test certificate when `https`.
    pub async fn start_with(ready: bool, https: bool) -> Web {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("warn")
            .with_test_writer()
            .try_init();
        pb_tls::install_default().unwrap();
        let fake = FakeFluxer::start(FakeConfig::default()).await;
        fake.add_user(ADA, "ada", Some("Ada"));
        fake.add_user(MAX, "max", None);
        fake.add_user(BEA, "bea", Some("Bea"));
        fake.add_guild(G, "Alpha", OWNER, EVERYONE);
        fake.add_role(G, MODS, "Mods", pb_fluxer_api::perms::MANAGE_GUILD);
        fake.add_channel(G, LOUNGE, "Lounge", 2);
        fake.add_member(G, ADA, &[MODS]);
        fake.add_member(G, MAX, &[]);
        fake.add_member(G, OWNER, &[]);
        fake.add_guild(G2, "Beta", BEA, EVERYONE);
        fake.add_member(G2, BEA, &[]);

        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_path_buf();
        let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&d.join("log")).unwrap().0);
        let index = Arc::new(TursoIndex::open(&d.join("index/index.db"), log.clone()).await.unwrap());
        let blobs = Arc::new(FsBlobStore::open(&d.join("blobs"), &d.join("tmp")).await.unwrap());
        let settings_files = Arc::new(FsSettingsFiles::new(&d.join("settings")));
        let secrets = Arc::new(FsSecretsFile::new(&d));
        let (mut tree, _) = settings_files.load().await.unwrap();
        if ready {
            let cfg = fake.config();
            secrets
                .save(&Secrets {
                    bot_token: Some(SecretString::from(cfg.token.clone())),
                    client_secret: Some(SecretString::from(cfg.client_secret.clone())),
                    setup: SetupState {
                        done: true,
                        owner: Some(UserId(OWNER)),
                        finished_at: Some(jiff::Timestamp::now()),
                    },
                    ..Secrets::default()
                })
                .await
                .unwrap();
            let changes: Vec<_> = tree
                .set(Scope::Global, SettingKey::Instance, serde_json::json!(fake.url()), true)
                .unwrap()
                .into_iter()
                .chain(tree.track(GuildId(G), UserId(MAX), None, jiff::Timestamp::now()))
                .collect();
            settings_files.write(&changes).await.unwrap();
        }
        let inference = Inference::start(Models {
            vad: Box::new(pb_vad_silero::EnergyVad::default()),
            classifier: Box::new(Quiet(ClassifierInfo {
                model: "quiet".into(),
                min_samples: 480,
                max_samples: 480_000,
                device: "none".into(),
            })),
            tts: None,
            tts_threads: 1,
        })
        .unwrap();
        let deps = Deps {
            fluxer: Arc::new(pb_fluxer::FluxerClient::default()),
            voice: Arc::new(NoVoice),
            inference: inference.clone(),
            log: log.clone(),
            index: index.clone(),
            blobs: blobs.clone(),
            settings_files,
            secrets: secrets.clone(),
            hub: pb_live::Hub::new(),
            clock: Arc::new(SystemClock::default()),
            version: "test".into(),
            shipped_clips: Vec::new(),
            disk_free: Arc::new(|| None),
        };
        let engine = Arc::new(Engine::start(deps, tree).await.unwrap());
        let (stop, stop_rx) = watch::channel(false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let scheme = if https { "https" } else { "http" };
        fake.add_redirect(&format!("{scheme}://{addr}/auth/callback"));
        let cert = std::fs::read(pb_testkit::fixture("tls/cert.pem")).unwrap();
        let key = std::fs::read(pb_testkit::fixture("tls/key.pem")).unwrap();
        let state = pb_web_server::WebState::new(
            pb_web_server::WebConfig {
                bind: addr,
                site_root: d.join("site"),
                live: pb_live::SessionCfg::default(),
                setup_code_file: d.join("setup-code").display().to_string(),
                token_from_env: false,
                client_secret_from_env: false,
                tls: https.then(|| pb_tls::server_config(&cert, &key).unwrap()),
            },
            pb_web_server::WebParts {
                engine: engine.clone(),
                index,
                blobs,
                log: log.clone(),
                secrets,
                sessions: Arc::new(FsSessionsFile::new(&d)),
                version: "test".into(),
                shutdown: stop_rx,
            },
        )
        .await
        .unwrap();
        tokio::spawn(pb_web_server::serve(state, listener));
        let mut http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if https {
            http = http.tls_backend_preconfigured((*pb_tls::pinned_client_config(&cert).unwrap()).clone());
        }
        let http = http.build().unwrap();
        let web = Web {
            fake,
            engine,
            base: format!("{scheme}://{addr}"),
            dir,
            http,
            log,
            stop,
            inference,
        };
        if ready {
            web.wait_ready().await;
        }
        web
    }

    pub async fn wait_ready(&self) {
        let end = std::time::Instant::now() + Duration::from_secs(20);
        let communities = || match self.engine.hub().state(&pb_live_proto::Topic::Sidebar) {
            Some(pb_live_proto::TopicState::Sidebar(s)) => s.communities.len(),
            _ => 0,
        };
        while self.engine.connection() != Connection::Ready || communities() < 2 {
            assert!(
                std::time::Instant::now() < end,
                "the bot did not log in: {:?}",
                self.engine.connection()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn get(&self, path: &str, cookie: Option<&str>) -> Got {
        self.send(self.http.get(self.url(path)), cookie).await
    }

    pub async fn post(&self, path: &str, cookie: Option<&str>, form: &[(&str, &str)]) -> Got {
        self.send(self.http.post(self.url(path)).form(form), cookie).await
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_owned()
        } else {
            format!("{}{path}", self.base)
        }
    }

    pub async fn send(&self, req: reqwest::RequestBuilder, cookie: Option<&str>) -> Got {
        let req = match cookie {
            Some(c) => req.header(COOKIE, c),
            None => req,
        };
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        let location = res.headers().get(LOCATION).map(|l| l.to_str().unwrap().to_owned());
        let csp = res
            .headers()
            .get("content-security-policy")
            .map(|l| l.to_str().unwrap().to_owned());
        let cookies = res
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|c| c.to_str().unwrap().to_owned())
            .collect();
        let body = res.text().await.unwrap_or_default();
        Got {
            status,
            location,
            csp,
            cookies,
            body,
        }
    }

    /// Logs in through the fake Fluxer as `user`; `Ok(cookie header)` or `Err(the notice shown)`.
    pub async fn login(&self, user: u64) -> Result<String, String> {
        self.login_with(user, None).await
    }

    /// The same, carrying extra cookies (the setup session).
    pub async fn login_with(&self, user: u64, extra: Option<&str>) -> Result<String, String> {
        self.fake.log_in_browser(Some(user));
        let start = self.get("/login", extra).await;
        assert_eq!(start.status, 303, "{start:?}");
        let oauth = start.cookie("pb_oauth").expect("the login sets pb_oauth");
        let at_fluxer = self.get(start.location.as_deref().unwrap(), None).await;
        assert_eq!(at_fluxer.status, 303, "{at_fluxer:?}");
        let mut jar = format!("pb_oauth={oauth}");
        if let Some(e) = extra {
            jar = format!("{jar}; {e}");
        }
        let back = self.get(at_fluxer.location.as_deref().unwrap(), Some(&jar)).await;
        assert_eq!(back.status, 303, "{back:?}");
        match back.cookie("pb_session") {
            Some(s) => Ok(format!("pb_session={s}")),
            None => {
                let notice = back.cookie("pb_notice").expect("a failed login leaves a notice");
                let page = self.get("/", Some(&format!("pb_notice={notice}"))).await;
                Err(page.body)
            }
        }
    }

    /// The form token of a login (read from a page).
    pub async fn page_csrf(&self, cookie: &str) -> String {
        Web::csrf(&self.get("/", Some(cookie)).await.body)
    }

    /// The form token on a page.
    pub fn csrf(page: &str) -> String {
        let at = page.find("name=\"csrf\" value=\"").expect("a form token on the page") + 19;
        page[at..].split('"').next().unwrap().to_owned()
    }

    pub async fn stop(self) {
        let _ = self.stop.send(true);
        self.engine.shutdown().await;
        let inference = self.inference.clone();
        let _ = tokio::task::spawn_blocking(move || inference.shutdown()).await;
    }
}
