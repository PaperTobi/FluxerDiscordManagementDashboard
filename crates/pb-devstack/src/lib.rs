//! A fake Fluxer instance (with OAuth logins), a local LiveKit server and two people talking in a call, plus a data
//! directory for `pb run`: for trying the bot and its web UI, and for the browser tests.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use pb_domain::{GuildId, Scope, UserId};
use pb_fluxer_fake::{FakeConfig, FakeFluxer};
use pb_settings::SettingKey;
use pb_store_api::{Secrets, SecretsFile, SettingsFiles, SetupState};
use pb_testkit::lk::{LiveKitServer, Person};
use secrecy::SecretString;

pub const GUILD: u64 = 111_111;
pub const VOICE: u64 = 222_222;
pub const TEXT: u64 = 333_333;
pub const ALICE: u64 = 444_444;
pub const BOB: u64 = 555_555;
/// View Channel, Send Messages, Attach Files, Connect, Speak.
const EVERYONE: u64 = (1 << 10) | (1 << 11) | (1 << 15) | (1 << 20) | (1 << 21);

/// How to start.
#[derive(Debug, Clone)]
pub struct Opts {
    /// The data directory to prepare for `pb run`.
    pub data: PathBuf,
    /// Where the fake Fluxer listens.
    pub fluxer: SocketAddr,
    /// Where the bot's web UI will listen.
    pub web: SocketAddr,
    /// The web UI's address as the browser sees it (for the login redirect).
    pub ui: String,
    /// The model weights.
    pub weights: PathBuf,
    /// A finished setup (token, client secret, tracked people) instead of the wizard.
    pub ready: bool,
    /// Alice and Bob talk in the call, in a loop.
    pub talk: bool,
}

/// The running stack (stops when dropped).
pub struct Stack {
    pub fake: FakeFluxer,
    pub lk: Arc<LiveKitServer>,
    talkers: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for Stack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stack")
            .field("fluxer", &self.fake.url())
            .finish_non_exhaustive()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        for t in &self.talkers {
            t.abort();
        }
    }
}

/// The workspace root (for the built site and the shipped clips).
pub fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

impl Stack {
    pub async fn start(opts: &Opts) -> Result<Stack> {
        let lk = Arc::new(LiveKitServer::start()?);
        let lk_url = lk.url();
        let fake = FakeFluxer::start(FakeConfig {
            grant: Arc::new(move |g, c, bot, conn| {
                (
                    lk_url.clone(),
                    pb_testkit::lk::mint(&format!("user_{bot}_{conn}"), &format!("guild_{g}_channel_{c}"), true),
                )
            }),
            bind: opts.fluxer,
            ..FakeConfig::default()
        })
        .await;
        let cfg = fake.config().clone();
        fake.add_user(ALICE, "alice", Some("Alice"));
        fake.add_user(BOB, "bob", Some("Bob"));
        fake.add_guild(GUILD, "Alpha", cfg.owner_id, EVERYONE);
        fake.add_channel(GUILD, VOICE, "Lounge", 2);
        fake.add_channel(GUILD, TEXT, "mod-log", 0);
        for u in [ALICE, BOB, cfg.owner_id] {
            fake.add_member(GUILD, u, &[]);
        }
        // The browser is logged in to Fluxer as the application's owner, and logins come back to the web UI.
        fake.log_in_browser(Some(cfg.owner_id));
        fake.add_redirect(&format!("{}/auth/callback", opts.ui.trim_end_matches('/')));
        prepare(opts, &fake).await?;
        let mut talkers = Vec::new();
        if opts.talk {
            for (user, clips, pause, delay) in [
                (ALICE, &["profane_1.wav", "benign_1.wav"][..], 7, 0),
                (BOB, &["benign_1.wav"][..], 9, 3),
            ] {
                let (lk, fake) = (lk.clone(), fake.clone());
                talkers.push(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    if let Err(e) = talk(&lk, &fake, user, clips, pause).await {
                        tracing::warn!(user, error = %e, "stopped talking");
                    }
                }));
            }
        }
        Ok(Stack { fake, lk, talkers })
    }
}

/// Writes `config.toml` (and with `--ready`, secrets and settings) into the data directory.
async fn prepare(opts: &Opts, fake: &FakeFluxer) -> Result<()> {
    let root = workspace_root();
    let d = &opts.data;
    std::fs::create_dir_all(d)?;
    let config = format!(
        "[web]\nbind = \"{}\"\nsite = \"{}\"\n\n[inference]\nweights = \"{}\"\nespeak_data = \"{}\"\nclips = \"{}\"\n",
        opts.web,
        root.join("target/site").display(),
        opts.weights.display(),
        pb_espeak::BUILD_DATA_DIR,
        root.join("clips").display(),
    );
    std::fs::write(d.join("config.toml"), config)?;
    if !opts.ready {
        return Ok(());
    }
    let cfg = fake.config();
    let secrets = pb_store::FsSecretsFile::new(d);
    let mut s = secrets.load().await.unwrap_or_default();
    s = Secrets {
        bot_token: Some(SecretString::from(cfg.token.clone())),
        client_secret: Some(SecretString::from(cfg.client_secret.clone())),
        setup: SetupState {
            done: true,
            owner: Some(UserId(cfg.owner_id)),
            finished_at: Some(jiff::Timestamp::now()),
        },
        ..s
    };
    secrets.save(&s).await?;
    let files = pb_store::FsSettingsFiles::new(&d.join("settings"));
    let (mut tree, _) = files.load().await?;
    let mut changes = Vec::new();
    let g = GuildId(GUILD);
    for (scope, key, value) in [
        (Scope::Global, SettingKey::Instance, serde_json::json!(fake.url())),
        (Scope::Global, SettingKey::UiUrl, serde_json::json!(opts.ui)),
        (
            Scope::Server { guild: g },
            SettingKey::ModlogChannel,
            serde_json::json!(TEXT.to_string()),
        ),
    ] {
        changes.extend(tree.set(scope, key, value, true)?);
    }
    for u in [ALICE, BOB] {
        changes.extend(tree.track(g, UserId(u), None, jiff::Timestamp::now()));
    }
    files.write(&changes).await?;
    Ok(())
}

/// `user` joins the call and says the fixtures in turn, `pause` seconds apart, for ever.
pub async fn talk(lk: &LiveKitServer, fake: &FakeFluxer, user: u64, clips: &[&str], pause: u64) -> Result<()> {
    let conn = fake.voice_join(GUILD, VOICE, user);
    let bot = fake.config().bot_id;
    let person = Person::join(
        lk,
        &format!("user_{user}_{conn}"),
        &format!("guild_{GUILD}_channel_{VOICE}"),
        &format!("user_{bot}_"),
    )
    .await?;
    let mic = person.publish_mic().await?;
    let audio: Vec<Vec<i16>> = clips
        .iter()
        .map(|c| {
            let (pcm, rate) = pb_testkit::audio::read_wav(&pb_testkit::fixture(c))?;
            pb_testkit::audio::resample(&pcm, rate, 48_000)
        })
        .collect::<Result<_>>()?;
    loop {
        for a in &audio {
            mic.say(a).await?;
            // Silence between sentences (a muted microphone still sends frames).
            mic.say(&vec![0; 48_000 * usize::try_from(pause).unwrap_or(5)]).await?;
        }
    }
}
