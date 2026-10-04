//! What every task shares.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use pb_domain::PlayPurpose;
use pb_domain::{Audience, BlobHash, ChannelId, GuildId, Lang, SentenceId, UserId};
use pb_fluxer_api::FluxerCtl;
use pb_infer::SpeakPriority;
use pb_policy::Chan;
use pb_store_api::{Actor, ClipRecord, Event, PlayRecord};
use pb_voicelines::{Fields, Line};
use tokio::sync::{mpsc, oneshot, watch};

use super::audio_cache::AudioCache;
use super::deps::Deps;
use super::guilds::Guilds;
use super::live::Live;
use super::mailbox::Addr;
use super::recorder::Recorder;
use super::settings::SettingsService;
use super::supervise::Supervisor;

/// Something to say in a call.
#[derive(Debug)]
pub struct PlayItem {
    pub line: Line,
    /// Who it is about (their languages, name, and the audience "offender").
    pub person: Option<UserId>,
    pub audience: Audience,
    pub fields: Fields,
    pub purpose: PlayPurpose,
    pub sentence: Option<SentenceId>,
    /// Not voiced after this (monotonic seconds): a warning that waited too long is recorded, not played.
    pub deadline: Option<f64>,
    pub by: Option<Actor>,
    /// Say exactly this (the "Say now" box) instead of resolving `line`.
    pub text: Option<(Lang, String)>,
    /// The language the person was heard speaking (for `voice_language = auto`).
    pub heard: Option<pb_domain::ClfLang>,
    /// The detection type `{label}` speaks (in the utterance's language).
    pub label: Option<pb_domain::Label>,
    pub done: Option<oneshot::Sender<PlayRecord>>,
}

/// How far the engine is in shutting down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Running,
    /// No new calls, microphones or chat commands; what was heard is still decided, said and reported.
    Stopping,
    /// Leave voice while the gateway is still open, close the rooms, then the gateway.
    Leaving,
}

impl Core {
    pub(super) fn running(&self) -> bool {
        *self.phase.borrow() == Phase::Running
    }
}

/// Commands to a room.
#[derive(Debug)]
pub enum RoomCmd {
    Play(Box<PlayItem>),
    /// Shutdown: stop listening, cut open speech and wait until every sentence is scored and handed on.
    Flush(oneshot::Sender<()>),
    /// Shutdown: wait until everything queued was said (or recorded as not said).
    Drain(oneshot::Sender<()>),
    /// Leave the room (answered once it is closed).
    Close(Option<oneshot::Sender<()>>),
}

/// The way to a room.
#[derive(Debug, Clone)]
pub struct RoomHandle {
    pub chan: Chan,
    pub tx: mpsc::UnboundedSender<RoomCmd>,
}

impl RoomHandle {
    pub fn play(&self, item: PlayItem) -> bool {
        self.tx.send(RoomCmd::Play(Box::new(item))).is_ok()
    }
}

/// A rendered phrase: (voice, speech rate in thousandths, text).
pub type SpeechKey = (String, u32, String);

/// A phrase's 48 kHz audio, or why it could not be rendered.
pub(super) type SpeechResult = Result<Arc<[i16]>, super::error::RenderError>;

/// A render in progress that later callers wait for.
pub(super) type Flight = Arc<tokio::sync::OnceCell<SpeechResult>>;

/// How the bot's Fluxer connection is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    NoToken,
    Connecting,
    Ready,
    /// Waiting before trying again.
    Retrying(String),
    /// The instance has voice turned off (looked at again now and then).
    NoVoice,
    TokenRejected,
    /// The gateway refused for good (sharding, protocol); needs an update or another setup.
    Stopped(String),
}

/// The Fluxer connection and the login attempt (restart request) it answers.
#[derive(Debug, Clone)]
pub struct Login {
    pub attempt: u64,
    pub state: Connection,
}

/// A person's names as recorded: user name, display name, nickname, avatar.
pub(super) type Names = (String, Option<String>, Option<String>, Option<String>);

/// The engine-wide state.
pub struct Core {
    pub deps: Deps,
    pub settings: SettingsService,
    pub(super) guilds: Published<Guilds>,
    pub(super) ctl: Published<Option<Arc<dyn FluxerCtl>>>,
    pub live: Live,
    /// The clip library (removed clips left out).
    pub(super) clips: Published<BTreeMap<BlobHash, ClipRecord>>,
    /// Rooms the bot is in.
    pub(super) rooms: Published<BTreeMap<Chan, RoomHandle>>,
    /// Running number of each person's sentences (for the conveyor).
    pub(super) sentence_no: Published<HashMap<(GuildId, UserId), u32>>,
    /// Rendered speech: (voice, rate in thousandths, text) → 48 kHz samples.
    pub speech: Mutex<AudioCache<SpeechKey>>,
    /// Speech being rendered right now, and for whom (a second caller waits for it instead of rendering again).
    pub(super) speech_inflight: Mutex<HashMap<SpeechKey, (SpeakPriority, Flight)>>,
    /// Decoded clip renders.
    pub clip_pcm: Mutex<AudioCache<BlobHash>>,
    pub moderation: Addr<super::moderation::ModMsg>,
    /// Who is in which voice channel (kept by the control actor).
    pub(super) voice: Published<pb_policy::VoiceWorld>,
    /// Swear-jar counts (seeded from the index, kept current by the moderation actor).
    pub(super) jar: Published<HashMap<(GuildId, UserId), u64>>,
    /// Timed mutes to lift (to the undo scheduler).
    pub undo: Addr<pb_store_api::ActionRecord>,
    /// Actions and reports for flagged sentences.
    pub(super) enforcer: Addr<super::enforcer::EnforcerMsg>,
    /// The clip played last per person and line (not repeated next time).
    pub no_repeat: Mutex<pb_voicelines::NoRepeat>,
    /// People whose microphone the bot listens to now.
    pub(super) listening: Published<std::collections::BTreeSet<(GuildId, UserId)>>,
    /// The names last recorded per person (and community): only changes are recorded again.
    pub(super) names_recorded: Published<BTreeMap<(UserId, Option<GuildId>), Names>>,
    /// Rooms where the bot may speak.
    pub(super) speaking: Published<std::collections::BTreeSet<Chan>>,
    /// The follow machine's connections and their states (for the community page).
    pub(super) conns: Published<BTreeMap<Chan, pb_live_proto::BotJoin>>,
    /// Live views to rebuild soon (to the views actor).
    pub(super) views: Addr<super::cells::Mark>,
    /// How the Fluxer connection is doing (the supervisor and the gateway session set it).
    pub connection: watch::Sender<Login>,
    /// The application's registered OAuth2 redirect addresses, and when Fluxer was last asked.
    pub redirects: Mutex<(Option<tokio::time::Instant>, Vec<String>)>,
    /// The event log's single writer.
    pub(super) recorder: Recorder,
    /// Runs the long-lived actors.
    pub(super) sup: Supervisor,
    /// Asks the gateway actor to log in again (the attempt number).
    pub(super) restart: watch::Sender<u64>,
    /// Running, or how far shutting down got.
    pub(super) phase: watch::Sender<Phase>,
    pub(super) started: jiff::Timestamp,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

/// State one part of the engine keeps and others read: readers take a snapshot that does not change (and hold no
/// lock while they use it) or follow changes; a change copies the value only while a reader still holds the old one.
pub struct Published<T>(watch::Sender<Arc<T>>);

impl<T> std::fmt::Debug for Published<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Published").finish_non_exhaustive()
    }
}

impl<T: Default> Default for Published<T> {
    fn default() -> Self {
        Published(watch::Sender::new(Arc::default()))
    }
}

impl<T: Clone> Published<T> {
    pub fn get(&self) -> Arc<T> {
        self.0.borrow().clone()
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut f = Some(f);
        let mut out = None;
        self.0.send_modify(|v| out = f.take().map(|f| f(Arc::make_mut(v))));
        match out {
            Some(r) => r,
            None => unreachable!("send_modify runs its closure once"),
        }
    }
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Core {
    /// Waits until the index holds every event logged so far (it applies them in the background).
    pub async fn index_caught_up(&self) {
        if let Some(head) = self.deps.log.head() {
            self.deps.index.caught_up(head.seq).await;
        }
    }

    /// The communities as last seen (a snapshot).
    pub fn guilds(&self) -> Arc<Guilds> {
        self.guilds.get()
    }

    pub fn update_guilds<R>(&self, f: impl FnOnce(&mut Guilds) -> R) -> R {
        self.guilds.update(f)
    }

    pub fn ctl(&self) -> Option<Arc<dyn FluxerCtl>> {
        (*self.ctl.get()).clone()
    }

    /// A picture's address on the instance's media server (the gateway sends only its hash).
    pub fn avatar_url(&self, user: UserId, hash: Option<&str>) -> Option<String> {
        let hash = hash?;
        self.ctl().and_then(|c| c.endpoints().avatar_url(user, Some(hash), 64))
    }

    pub fn set_ctl(&self, ctl: Option<Arc<dyn FluxerCtl>>) {
        if let Ok(mut r) = self.redirects.lock() {
            *r = (None, Vec::new());
        }
        self.ctl.update(|c| *c = ctl);
    }

    pub fn room(&self, chan: Chan) -> Option<RoomHandle> {
        self.rooms.get().get(&chan).cloned()
    }

    pub fn rooms(&self) -> Vec<RoomHandle> {
        self.rooms.get().values().cloned().collect()
    }

    pub fn room_of_channel(&self, guild: GuildId, channel: ChannelId) -> Option<RoomHandle> {
        self.room(Chan { guild, channel })
    }

    pub fn set_room(&self, chan: Chan, h: Option<RoomHandle>) {
        self.rooms.update(|rooms| match h {
            Some(h) => {
                rooms.insert(chan, h);
            }
            None => {
                rooms.remove(&chan);
            }
        });
    }

    /// Who is in which call (a snapshot).
    pub fn voice(&self) -> Arc<pb_policy::VoiceWorld> {
        self.voice.get()
    }

    pub fn update_voice<R>(&self, f: impl FnOnce(&mut pb_policy::VoiceWorld) -> R) -> R {
        self.voice.update(f)
    }

    pub fn is_listening(&self, guild: GuildId, user: UserId) -> bool {
        self.listening.get().contains(&(guild, user))
    }

    pub fn set_listening(&self, guild: GuildId, user: UserId, on: bool) {
        self.listening.update(|l| {
            if on {
                l.insert((guild, user));
            } else {
                l.remove(&(guild, user));
            }
        });
        self.mark_person(guild, user);
    }

    pub fn set_speaking(&self, chan: Chan, on: bool) {
        self.speaking.update(|s| {
            if on {
                s.insert(chan);
            } else {
                s.remove(&chan);
            }
        });
    }

    /// The community's live views need rebuilding (its page and its sidebar entry).
    pub fn mark_guild(&self, guild: GuildId) {
        let _ = self.views.send(super::cells::Mark::Guild(guild));
    }

    /// A person's live views need rebuilding (their page, their community).
    pub fn mark_person(&self, guild: GuildId, user: UserId) {
        let _ = self.views.send(super::cells::Mark::Person(guild, user));
    }

    pub fn mark_all(&self) {
        let _ = self.views.send(super::cells::Mark::All);
    }

    pub fn jar(&self, guild: GuildId, user: UserId) -> u64 {
        self.jar.get().get(&(guild, user)).copied().unwrap_or(0)
    }

    pub fn clip(&self, h: &BlobHash) -> Option<ClipRecord> {
        self.clips.get().get(h).cloned()
    }

    pub fn put_clip(&self, c: ClipRecord) {
        self.clips.update(|cs| cs.insert(c.render, c));
    }

    pub fn remove_clip(&self, h: &BlobHash) {
        self.clips.update(|cs| cs.remove(h));
        lock(&self.clip_pcm).remove(h);
    }

    pub fn next_sentence_no(&self, guild: GuildId, user: UserId) -> u32 {
        self.sentence_no.update(|m| {
            let n = m.entry((guild, user)).or_insert(0);
            *n += 1;
            *n
        })
    }

    /// The bot owner (the application's owner).
    pub fn owner(&self) -> Option<UserId> {
        self.ctl().and_then(|c| c.me().owner)
    }

    pub fn bot(&self) -> Option<UserId> {
        self.ctl().map(|c| c.me().user.id)
    }

    /// Records events after everything handed over before, without waiting for the disk (see [`Recorder`]).
    pub fn record(&self, events: Vec<Event>) {
        self.recorder.record(events);
    }

    /// Records events and waits until they are on disk; `false` when that failed (the recorder logged why).
    pub async fn record_durably(&self, events: Vec<Event>) -> bool {
        self.recorder.record_acked(events).await.is_ok()
    }
}
