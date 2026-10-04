//! The engine: starts the actors, keeps a Fluxer session alive (a new one after a token or instance change), and is
//! the API the web UI and the binary use.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use pb_domain::PlayPurpose;
use pb_domain::{Audience, ChannelId, GuildId, Lang, UserId};
use pb_fluxer_api::{Fatal, LoginError};
use pb_live_proto::FluxerState;
use pb_policy::{Chan, VoiceWorld};
use pb_settings::SettingsTree;
use pb_store_api::{Actor, Event, PlayRecord, Started, Stopped, StoreError};
use pb_voicelines::{Fields, Line};
use tokio::sync::{mpsc, oneshot, watch};

use super::control::{self, SessionEnd};
use super::core::{Connection, Core, Login, PlayItem, Snapshot};
use super::deps::Deps;
use super::error::EngineError;
use super::guilds::Guilds;
use super::live::Live;
use super::settings::SettingsService;

/// The running bot.
pub struct Engine {
    pub(super) core: Arc<Core>,
    restart: watch::Sender<u64>,
    stop: watch::Sender<bool>,
    supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("connection", &self.connection())
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Starts everything with the settings `tree` (already loaded from the files).
    pub async fn start(deps: Deps, tree: SettingsTree) -> Result<Engine, StoreError> {
        let settings = SettingsService::new(tree, deps.settings_files.clone(), deps.log.clone());
        let (mod_tx, mod_rx) = mpsc::unbounded_channel();
        let (undo_tx, undo_rx) = mpsc::unbounded_channel();
        let live = Live::new(deps.hub.clone());
        let core = Arc::new(Core {
            settings,
            guilds: Snapshot::default(),
            ctl: RwLock::new(None),
            live,
            clips: RwLock::new(Default::default()),
            rooms: RwLock::new(Default::default()),
            sentence_no: Mutex::new(HashMap::new()),
            speech: Mutex::new(HashMap::new()),
            clip_pcm: Mutex::new(HashMap::new()),
            moderation: mod_tx,
            voice: Snapshot::default(),
            jar: Mutex::new(HashMap::new()),
            undo: undo_tx,
            no_repeat: Mutex::new(Default::default()),
            listening: Mutex::new(Default::default()),
            speaking: Mutex::new(Default::default()),
            conns: RwLock::new(Default::default()),
            dirty: Mutex::new(Default::default()),
            redirects: Mutex::new((None, Vec::new())),
            connection: watch::channel(Login {
                attempt: 0,
                state: Connection::NoToken,
            })
            .0,
            deps,
        });
        // What the log already knows: the clip library, swear jars, violations for the escalation counts, mutes to lift.
        let index = core.deps.index.clone();
        for c in index.clips().await? {
            core.put_clip(c.record);
        }
        // The names last seen, for communities and tracked people the gateway has not mentioned (yet).
        let communities = index.communities().await?;
        core.update_guilds(|gs| {
            gs.known_communities = communities.into_iter().map(|c| (c.guild, (c.name, c.icon))).collect();
        });
        let tree0 = core.settings.current();
        for g in tree0.known_guilds() {
            let users: Vec<UserId> = tree0.listed_for(g).into_iter().collect();
            if users.is_empty() {
                continue;
            }
            for p in index.people(&users, Some(g)).await? {
                core.update_guilds(|gs| {
                    gs.remember(
                        g,
                        super::guilds::Person {
                            user: p.user,
                            username: p.username,
                            display_name: p.display_name,
                            nick: p.nick,
                            avatar: p.avatar,
                            roles: Vec::new(),
                            bot: false,
                        },
                    );
                });
            }
        }
        drop(tree0);
        let jars = index.jar(None).await?;
        if let Ok(mut j) = core.jar.lock() {
            for r in jars {
                j.insert((r.guild, r.user), r.count);
            }
        }
        // Every violation so far: any community or person may count over any window, and windows can be lengthened
        // later (one number per violation).
        let times = index.violation_times().await?;
        let _ = core.moderation.send(super::moderation::ModMsg::Seed(times));
        let pending = index.pending_undos().await?;
        tokio::spawn(super::moderation::run(core.clone(), mod_rx));
        tokio::spawn(super::actions::undo_scheduler(core.clone(), pending, undo_rx));
        tokio::spawn(super::reports::digest_scheduler(core.clone()));
        tokio::spawn(follow_threads(core.clone()));
        core.mark_all();
        tokio::spawn(super::cells::refresh(core.clone()));
        let started = core.deps.clock.now();
        core.record(vec![Event::Started(Started {
            version: core.deps.version.clone(),
        })])
        .await;
        let (restart, restart_rx) = watch::channel(0);
        let (stop, stop_rx) = watch::channel(false);
        let sup = tokio::spawn(supervise(core.clone(), restart_rx, stop_rx));
        tokio::spawn(system_status(core.clone(), started));
        Ok(Engine {
            core,
            restart,
            stop,
            supervisor: Mutex::new(Some(sup)),
        })
    }

    /// After the event log stopped writing (a full disk …): tries writing again (the System page).
    pub async fn retry_log(&self) -> Result<(), EngineError> {
        self.core.deps.log.retry().await?;
        Ok(())
    }

    /// Reads the settings files and the voice files again (SIGHUP, the System page). Problems in the files are
    /// returned (their old values stay in use).
    pub async fn reload(&self) -> Result<Vec<pb_settings::FileError>, super::settings::ChangeError> {
        let problems = self.core.settings.reload().await?;
        let threads = self
            .core
            .settings
            .current()
            .effective(None, None)
            .tts_threads
            .value
            .get() as usize;
        match self.core.deps.inference.reload_tts(threads).await {
            Ok(voices) => tracing::info!(voices = voices.len(), "voices read again"),
            Err(e) => tracing::warn!(error = %e, "the voices could not be read again"),
        }
        Ok(problems)
    }

    /// Whether `uri` is registered as an OAuth2 redirect address of the bot's application: `Err` with the registered
    /// ones when it is not, `None` while that is unknown (not connected). Fluxer is asked again at most every few
    /// seconds, so anonymous login attempts cannot spend the bot's rate limits.
    pub async fn redirect_registered(&self, uri: &str) -> Option<Result<(), Vec<String>>> {
        const ASK_AGAIN: Duration = Duration::from_secs(5);
        let known = |list: &[String]| list.iter().any(|r| r == uri);
        let fresh = {
            let r = self.core.redirects.lock().ok()?;
            if known(&r.1) {
                return Some(Ok(()));
            }
            r.0.is_some_and(|at| at.elapsed() < ASK_AGAIN).then(|| r.1.clone())
        };
        let list = match fresh {
            Some(list) => list,
            None => {
                let list = self.core.ctl()?.application().await.ok()?.redirect_uris;
                if let Ok(mut r) = self.core.redirects.lock() {
                    *r = (Some(std::time::Instant::now()), list.clone());
                }
                list
            }
        };
        Some(if known(&list) { Ok(()) } else { Err(list) })
    }

    /// A model thread that ended or hangs (the process should be restarted).
    pub fn stuck_model(&self) -> Option<pb_infer::Worker> {
        self.core.deps.inference.stuck()
    }

    /// The bot's permissions in a community (and channel).
    pub fn bot_permissions(&self, guild: GuildId, channel: Option<ChannelId>) -> u64 {
        self.core.guilds().bot_permissions(guild, channel)
    }

    /// A community the bot is in or has settings for.
    pub fn knows_guild(&self, guild: GuildId) -> bool {
        super::cells::communities(&self.core).contains(&guild)
    }

    /// A person's name and picture as pages show them.
    pub fn who(&self, guild: GuildId, user: UserId) -> pb_live_proto::Who {
        super::cells::who(&self.core, guild, user)
    }

    /// Whether a person is tracked in a community (and how).
    pub fn tracking(&self, guild: GuildId, user: UserId) -> pb_live_proto::Tracking {
        super::cells::tracking(&self.core, guild, user)
    }

    /// A person's live view for a page: built (with counts and latest sentences) when nobody watched it yet.
    pub async fn person_view(&self, guild: GuildId, user: UserId) -> Option<pb_live_proto::PersonState> {
        let topic = pb_live_proto::Topic::Person { guild, user };
        if !self.core.live.hub.has(&topic) {
            use pb_live::CellSource as _;
            if !self.cells().ensure(&topic) {
                return None;
            }
            super::cells::fill_person(&self.core, guild, user).await;
        }
        match self.core.live.hub.state(&topic) {
            Some(pb_live_proto::TopicState::Person(s)) => Some(*s),
            _ => None,
        }
    }

    /// Builds live cells on demand (for the web UI's live connections).
    pub fn cells(&self) -> super::cells::Cells {
        super::cells::Cells {
            core: self.core.clone(),
        }
    }

    pub fn hub(&self) -> &pb_live::Hub {
        &self.core.live.hub
    }

    pub fn settings(&self) -> &SettingsService {
        &self.core.settings
    }

    pub fn connection(&self) -> Connection {
        self.core.connection.borrow().state.clone()
    }

    /// Logs in again (after the token or the instance changed). Returns the attempt to wait for with
    /// [`Engine::login_outcome`].
    pub fn reconnect(&self) -> u64 {
        let mut attempt = 0;
        self.restart.send_modify(|n| {
            *n += 1;
            attempt = *n;
        });
        attempt
    }

    /// Waits until login attempt `attempt` (from [`Engine::reconnect`]) has an outcome or `timeout` passed; returns the
    /// connection state then.
    pub async fn login_outcome(&self, attempt: u64, timeout: Duration) -> Connection {
        let mut rx = self.core.connection.subscribe();
        let answered = rx.wait_for(|l| l.attempt >= attempt && l.state != Connection::Connecting);
        match tokio::time::timeout(timeout, answered).await {
            Ok(Ok(l)) => l.state.clone(),
            _ => self.connection(),
        }
    }

    /// Looks up a Fluxer instance's endpoints (to check an address before it is saved).
    pub async fn discover(&self, instance: &url::Url) -> Result<pb_fluxer_api::Endpoints, String> {
        self.core
            .deps
            .fluxer
            .discover(instance)
            .await
            .map_err(|e| e.to_string())
    }

    /// The communities and who is where (for the web UI).
    pub fn guilds(&self) -> Arc<Guilds> {
        self.core.guilds()
    }

    pub fn voice(&self) -> Arc<VoiceWorld> {
        self.core.voice()
    }

    /// The bot's own user and the application owner, once logged in.
    pub fn identity(&self) -> Option<pb_fluxer_api::BotIdentity> {
        self.core.ctl().map(|c| c.me())
    }

    pub fn endpoints(&self) -> Option<pb_fluxer_api::Endpoints> {
        self.core.ctl().map(|c| c.endpoints().clone())
    }

    /// The installed text-to-speech voices.
    pub fn voices(&self) -> Vec<pb_models_api::VoiceInfo> {
        self.core.deps.inference.voices()
    }

    /// Rooms the bot is in.
    pub fn rooms(&self) -> Vec<Chan> {
        self.core.rooms().into_iter().map(|r| r.chan).collect()
    }

    /// Says something now in a call ("Say now"). `person`: who it is for (their voice and name); `text` in `lang`, or
    /// the voice line `line`. `Err` unless it reached them.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn say(
        &self,
        guild: GuildId,
        channel: ChannelId,
        person: Option<UserId>,
        line: Line,
        text: Option<(Lang, String)>,
        audience: Audience,
        by: Actor,
    ) -> Result<PlayRecord, EngineError> {
        let room = self
            .core
            .room_of_channel(guild, channel)
            .ok_or(EngineError::BotNotInCall)?;
        let (tx, rx) = oneshot::channel();
        let ok = room.play(PlayItem {
            line,
            person,
            audience,
            fields: Fields::new(),
            purpose: PlayPurpose::Say,
            sentence: None,
            deadline: None,
            by: Some(by),
            text,
            heard: None,
            label: None,
            done: Some(tx),
        });
        if !ok {
            return Err(EngineError::BotNotInCall);
        }
        let record = rx.await.map_err(|_| EngineError::BotNotInCall)?;
        if record.ok() {
            Ok(record)
        } else {
            Err(EngineError::NotSaid(record.outcome))
        }
    }

    /// Empties a person's swear jar.
    pub async fn reset_jar(&self, guild: GuildId, user: UserId, by: Actor) {
        super::commands::reset_jar(&self.core, guild, user, by).await;
    }

    /// Sends the summary report now (covering the time since the last one).
    pub async fn send_digest(&self) -> Result<bool, EngineError> {
        super::reports::digest_once(&self.core, true).await
    }

    /// Renders a line for a preview (48 kHz samples and what it says).
    pub async fn preview(
        &self,
        guild: GuildId,
        person: Option<UserId>,
        line: Line,
        text: Option<(Lang, String)>,
    ) -> Result<super::speak::Rendered, EngineError> {
        super::speak::render(
            &self.core,
            guild,
            None,
            person,
            &line,
            text.as_ref(),
            None,
            None,
            &Fields::new(),
            pb_infer::SpeakPriority::Preview,
        )
        .await
        .map_err(EngineError::Render)
    }

    /// The Fluxer endpoints for a web login: the connected ones, or freshly discovered (logging in to the web UI works
    /// while the bot itself cannot connect, so a rejected token can be replaced).
    pub async fn login_endpoints(&self) -> Result<pb_fluxer_api::Endpoints, String> {
        if let Some(ep) = self.endpoints() {
            return Ok(ep);
        }
        let instance = self
            .core
            .settings
            .current()
            .effective(None, None)
            .instance
            .value
            .url()
            .clone();
        self.core
            .deps
            .fluxer
            .discover(&instance)
            .await
            .map_err(|e| e.to_string())
    }

    /// Finishes a web login with Fluxer (exchanges the code, reads who logged in).
    pub async fn oauth_user(
        &self,
        ep: &pb_fluxer_api::Endpoints,
        client: &pb_fluxer_api::OAuthClient,
        code: &str,
        verifier: &str,
    ) -> Result<pb_fluxer_api::OAuthUser, String> {
        self.core
            .deps
            .fluxer
            .oauth_user(ep, client, code, verifier)
            .await
            .map_err(|e| e.to_string())
    }

    /// Whether `user` is the bot owner (the application's owner or one of the extra bot owners).
    pub fn is_owner(&self, user: UserId) -> bool {
        self.core.owner() == Some(user)
            || self
                .core
                .settings
                .current()
                .effective(None, None)
                .admin_user_ids
                .value
                .contains(&user)
    }

    /// The communities `user` administers (the chat commands' rule): owner, Administrator or Manage community, or an
    /// admin role — in allowed communities, and never where they are tracked.
    pub async fn admin_guilds(&self, user: UserId) -> std::collections::BTreeSet<GuildId> {
        let mut out = std::collections::BTreeSet::new();
        let guilds: Vec<GuildId> = self.core.guilds().available().into_iter().collect();
        for g in guilds {
            let tree = self.core.settings.current();
            if !tree.guild_allowed(g) {
                continue;
            }
            let known = self
                .core
                .guilds()
                .get(g)
                .map(|i| (i.owner, i.people.get(&user).map(|p| p.roles.clone())));
            let Some((owner, roles)) = known else { continue };
            let roles = match roles {
                Some(r) => r,
                None if owner == Some(user) => Vec::new(),
                None => match self.core.ctl() {
                    Some(ctl) => match ctl.member(g, user).await {
                        Ok(Some(m)) => m.roles,
                        _ => continue,
                    },
                    None => continue,
                },
            };
            let admin_roles = tree.effective(Some(g), None).admin_role_ids.value;
            let who = pb_commands::Author {
                operator: false,
                community_owner: owner == Some(user),
                admin_role: roles.iter().any(|r| admin_roles.contains(r)),
                manages: self.core.guilds().permissions(g, user, &roles, None)
                    & (pb_fluxer_api::perms::ADMINISTRATOR | pb_fluxer_api::perms::MANAGE_GUILD)
                    != 0,
                tracked_here: tree.listed_for(g).contains(&user),
            };
            if pb_commands::level(who) >= pb_commands::Level::Admin {
                out.insert(g);
            }
        }
        out
    }

    /// Records an event (web logins, uploads).
    pub async fn record(&self, events: Vec<Event>) -> bool {
        self.core.record(events).await
    }

    /// Stops: leaves voice, closes the gateway, records the stop.
    pub async fn shutdown(&self) {
        self.stop.send_replace(true);
        let sup = self.supervisor.lock().ok().and_then(|mut s| s.take());
        if let Some(s) = sup {
            let _ = tokio::time::timeout(Duration::from_secs(10), s).await;
        }
        self.core.record(vec![Event::Stopped(Stopped { clean: true })]).await;
    }
}

/// The System page: models, queues, storage, connection (only computed while someone watches it).
async fn system_status(core: Arc<Core>, started: jiff::Timestamp) {
    let topic = pb_live_proto::Topic::System;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if core.deps.hub.has(&topic) && !core.deps.hub.watched(&topic) {
            continue;
        }
        let inf = core.deps.inference.status();
        let conn = core.connection.borrow().state.clone();
        let fluxer = match conn {
            Connection::NoToken => FluxerState::NoToken,
            Connection::Connecting => FluxerState::Connecting,
            Connection::TokenRejected => FluxerState::TokenRejected,
            Connection::Retrying(e) => FluxerState::Reconnecting { error: e },
            Connection::NoVoice => FluxerState::NoVoice,
            Connection::Stopped(e) => FluxerState::Stopped { error: e },
            Connection::Ready => match core.ctl() {
                Some(c) => {
                    let me = c.me();
                    FluxerState::Ready {
                        bot: pb_live_proto::Who {
                            user: me.user.id,
                            name: me.user.shown().to_owned(),
                            avatar: core.avatar_url(me.user.id, me.user.avatar.as_deref()),
                        },
                    }
                }
                None => FluxerState::Connecting,
            },
        };
        let health = core.deps.log.health();
        let applied = core.deps.index.applied();
        let head = core.deps.log.head().map_or(0, |h| h.seq);
        let state = pb_live_proto::SystemState {
            version: core.deps.version.clone(),
            started_ms: started.as_millisecond(),
            fluxer,
            models: {
                use pb_infer::Worker;
                use pb_live_proto::{Model, ModelProblem, ModelStatus};
                let stuck = core.deps.inference.stuck();
                let problem = |w: Worker| (stuck == Some(w)).then_some(ModelProblem::NotAnswering);
                vec![
                    ModelStatus {
                        model: Model::Classifier,
                        device: Some(inf.classifier_device.clone()),
                        problem: problem(Worker::Classifier),
                    },
                    ModelStatus {
                        model: Model::VoiceActivity,
                        device: None,
                        problem: problem(Worker::Vad),
                    },
                    ModelStatus {
                        model: Model::Speech,
                        device: None,
                        problem: problem(Worker::Speech).or((inf.voices == 0).then_some(ModelProblem::NoVoices)),
                    },
                ]
            },
            queues: vec![
                pb_live_proto::QueueStatus {
                    queue: pb_live_proto::Queue::Scoring,
                    waiting: inf.classify.waiting,
                    done: inf.classify.done,
                    oldest_ms: inf.classify.oldest_ms,
                },
                pb_live_proto::QueueStatus {
                    queue: pb_live_proto::Queue::Speech,
                    waiting: inf.speak.waiting,
                    done: inf.speak.done,
                    oldest_ms: inf.speak.oldest_ms,
                },
            ],
            storage: pb_live_proto::StorageStatus {
                log_bytes: core.deps.log.size(),
                blob_bytes: core.deps.blobs.size(),
                free_bytes: (core.deps.disk_free)().unwrap_or(0),
                index_behind: head.saturating_sub(applied),
                index_problem: core.deps.index.problem(),
                index_skipped: core.deps.index.skipped().await.unwrap_or(0),
                log_halted: match health {
                    pb_store_api::WriterHealth::Ok => None,
                    pb_store_api::WriterHealth::Halted { error, .. } => Some(error),
                },
            },
            rooms: u32::try_from(core.rooms().len()).unwrap_or(u32::MAX),
            streams: u32::try_from(inf.vad_streams).unwrap_or(u32::MAX),
        };
        core.live.system(state);
    }
}

fn set(core: &Core, c: Connection) {
    core.connection.send_modify(|l| l.state = c);
}

/// Keeps one Fluxer session running: no token → wait; bad token → wait for a new one; unreachable → try again.
async fn supervise(core: Arc<Core>, mut restart: watch::Receiver<u64>, mut stop: watch::Receiver<bool>) {
    let mut failures = 0u32;
    loop {
        if *stop.borrow() {
            return;
        }
        // Every state set from here on answers the restart requests seen so far.
        let answering = *restart.borrow_and_update();
        core.connection.send_modify(|l| {
            l.state = Connection::Connecting;
            l.attempt = answering;
        });
        // Waits for a restart request or the stop (`true` = stop).
        let wait = |restart: &mut watch::Receiver<u64>, stop: &mut watch::Receiver<bool>, d: Option<Duration>| {
            let (mut r, mut s) = (restart.clone(), stop.clone());
            async move {
                let sleep = async {
                    match d {
                        Some(d) => tokio::time::sleep(d).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = r.changed() => false,
                    _ = s.changed() => true,
                    () = sleep => false,
                }
            }
        };
        let secrets = match core.deps.secrets.load().await {
            Ok(s) => s,
            Err(e) => {
                set(&core, Connection::Retrying(e.to_string()));
                if wait(&mut restart, &mut stop, Some(Duration::from_secs(30))).await {
                    return;
                }
                continue;
            }
        };
        let Some(token) = secrets.bot_token else {
            set(&core, Connection::NoToken);
            if wait(&mut restart, &mut stop, None).await {
                return;
            }
            continue;
        };
        set(&core, Connection::Connecting);
        let instance = core
            .settings
            .current()
            .effective(None, None)
            .instance
            .value
            .url()
            .clone();
        let retry = |e: String, failures: u32| {
            let d = Duration::from_secs((5u64 << failures.min(6)).min(300));
            (e, d)
        };
        let ep = match core.deps.fluxer.discover(&instance).await {
            Ok(ep) => ep,
            Err(e) => {
                failures += 1;
                let (msg, d) = retry(e.to_string(), failures);
                tracing::warn!(error = %msg, "Fluxer discovery failed; trying again in {:?}", d);
                set(&core, Connection::Retrying(msg.clone()));
                if wait(&mut restart, &mut stop, Some(d)).await {
                    return;
                }
                continue;
            }
        };
        if !ep.voice_enabled {
            tracing::error!("this Fluxer instance has voice turned off; looking again in 5 minutes");
            set(&core, Connection::NoVoice);
            if wait(&mut restart, &mut stop, Some(Duration::from_secs(300))).await {
                return;
            }
            continue;
        }
        let (ctl, events) = match core.deps.fluxer.login(&ep, &token).await {
            Ok(x) => x,
            Err(LoginError::TokenRejected) => {
                tracing::error!("Fluxer rejected the bot token; waiting for a new one");
                set(&core, Connection::TokenRejected);
                if wait(&mut restart, &mut stop, None).await {
                    return;
                }
                continue;
            }
            Err(e) => {
                failures += 1;
                let (msg, d) = retry(e.to_string(), failures);
                tracing::warn!(error = %msg, "could not log in to Fluxer; trying again in {:?}", d);
                set(&core, Connection::Retrying(msg.clone()));
                if wait(&mut restart, &mut stop, Some(d)).await {
                    return;
                }
                continue;
            }
        };
        failures = 0;
        core.set_ctl(Some(ctl.clone()));
        set(&core, Connection::Ready);
        let session = control::run(core.clone(), ctl.clone(), events);
        tokio::pin!(session);
        let mut instance_watch = core.settings.watch();
        let ended = loop {
            tokio::select! {
                end = &mut session => break Some(end),
                _ = restart.changed() => {
                    ctl.close().await;
                    break None;
                }
                _ = stop.changed() => {
                    ctl.close().await;
                    let _ = tokio::time::timeout(Duration::from_secs(5), &mut session).await;
                    core.set_ctl(None);
                    return;
                }
                _ = instance_watch.changed() => {
                    // A new instance address applies by logging in again.
                    if core.settings.current().effective(None, None).instance.value.url() != &instance {
                        ctl.close().await;
                        break None;
                    }
                }
            }
        };
        if ended.is_none() {
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut session).await;
        }
        core.set_ctl(None);
        match ended {
            Some(SessionEnd::Fatal(Fatal::TokenRejected)) => {
                set(&core, Connection::TokenRejected);
                if wait(&mut restart, &mut stop, None).await {
                    return;
                }
            }
            Some(SessionEnd::Fatal(f)) => {
                let msg = format!("{f:?}");
                tracing::error!(reason = %msg, "the Fluxer gateway refused the bot for good");
                set(&core, Connection::Stopped(msg));
                if wait(&mut restart, &mut stop, None).await {
                    return;
                }
            }
            Some(SessionEnd::Closed) | None => {}
        }
    }
}

/// The models' thread counts follow the settings (they were set when the models loaded).
async fn follow_threads(core: Arc<Core>) {
    let threads = |core: &Core| {
        let eff = core.settings.current().effective(None, None);
        (eff.cpu_threads.value.get(), eff.tts_threads.value.get())
    };
    let mut applied = threads(&core);
    let mut changed = core.settings.watch();
    while changed.changed().await.is_ok() {
        let (cpu, tts) = threads(&core);
        if cpu != applied.0
            && let Some(n) = std::num::NonZeroUsize::new(cpu as usize)
        {
            core.deps.inference.set_classifier_threads(n);
        }
        if tts != applied.1
            && let Err(e) = core.deps.inference.reload_tts(tts as usize).await
        {
            tracing::warn!(error = %e, "text-to-speech could not restart with {tts} threads");
        }
        applied = (cpu, tts);
    }
}
