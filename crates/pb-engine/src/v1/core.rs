//! What every task shares.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use pb_domain::PlayPurpose;
use pb_domain::{Audience, BlobHash, ChannelId, GuildId, Lang, SentenceId, UserId};
use pb_fluxer_api::FluxerCtl;
use pb_policy::Chan;
use pb_store_api::{Actor, ClipRecord, Event, NewEvent, PlayRecord};
use pb_voicelines::{Fields, Line};
use tokio::sync::{mpsc, oneshot, watch};

use super::audio_cache::AudioCache;
use super::deps::Deps;
use super::guilds::Guilds;
use super::live::Live;
use super::settings::SettingsService;

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

/// Commands to a room.
#[derive(Debug)]
pub enum RoomCmd {
    Play(Box<PlayItem>),
    Close,
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
    pub(super) guilds: Snapshot<Guilds>,
    pub ctl: RwLock<Option<Arc<dyn FluxerCtl>>>,
    pub live: Live,
    /// The clip library (removed clips left out).
    pub clips: RwLock<BTreeMap<BlobHash, ClipRecord>>,
    /// Rooms the bot is in.
    pub rooms: RwLock<BTreeMap<Chan, RoomHandle>>,
    /// Running number of each person's sentences (for the conveyor).
    pub sentence_no: Mutex<HashMap<(GuildId, UserId), u32>>,
    /// Rendered speech: (voice, rate in thousandths, text) → 48 kHz samples.
    pub speech: Mutex<AudioCache<SpeechKey>>,
    /// Decoded clip renders.
    pub clip_pcm: Mutex<AudioCache<BlobHash>>,
    pub moderation: mpsc::UnboundedSender<super::moderation::ModMsg>,
    /// Who is in which voice channel (kept by the control actor).
    pub(super) voice: Snapshot<pb_policy::VoiceWorld>,
    /// Swear-jar counts (seeded from the index, kept current by the moderation actor).
    pub jar: Mutex<HashMap<(GuildId, UserId), u64>>,
    /// Timed mutes to lift (to the undo scheduler).
    pub undo: mpsc::UnboundedSender<pb_store_api::ActionRecord>,
    /// The clip played last per person and line (not repeated next time).
    pub no_repeat: Mutex<pb_voicelines::NoRepeat>,
    /// People whose microphone the bot listens to now.
    pub listening: Mutex<std::collections::BTreeSet<(GuildId, UserId)>>,
    /// The names last recorded per person (and community): only changes are recorded again.
    pub(super) names_recorded: Mutex<BTreeMap<(UserId, Option<GuildId>), Names>>,
    /// Rooms where the bot may speak.
    pub speaking: Mutex<std::collections::BTreeSet<Chan>>,
    /// The follow machine's connections and their states (for the community page).
    pub conns: RwLock<BTreeMap<Chan, pb_live_proto::BotJoin>>,
    /// Live cells to rebuild soon.
    pub dirty: Mutex<super::cells::Dirty>,
    /// How the Fluxer connection is doing (the supervisor and the gateway session set it).
    pub connection: watch::Sender<Login>,
    /// The application's registered OAuth2 redirect addresses, and when Fluxer was last asked.
    pub redirects: Mutex<(Option<std::time::Instant>, Vec<String>)>,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

/// A value read often and changed seldom: readers get a snapshot and hold no lock while they use it (so one task can
/// read twice in a row without deadlocking against a waiting writer); writers copy it when a reader still has one.
#[derive(Debug, Default)]
pub struct Snapshot<T>(RwLock<Arc<T>>);

impl<T: Clone> Snapshot<T> {
    pub fn get(&self) -> Arc<T> {
        read(&self.0).clone()
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(Arc::make_mut(&mut write(&self.0)))
    }
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Core {
    /// The communities as last seen (a snapshot).
    pub fn guilds(&self) -> Arc<Guilds> {
        self.guilds.get()
    }

    pub fn update_guilds<R>(&self, f: impl FnOnce(&mut Guilds) -> R) -> R {
        self.guilds.update(f)
    }

    pub fn ctl(&self) -> Option<Arc<dyn FluxerCtl>> {
        read(&self.ctl).clone()
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
        *write(&self.ctl) = ctl;
    }

    pub fn room(&self, chan: Chan) -> Option<RoomHandle> {
        read(&self.rooms).get(&chan).cloned()
    }

    pub fn rooms(&self) -> Vec<RoomHandle> {
        read(&self.rooms).values().cloned().collect()
    }

    pub fn room_of_channel(&self, guild: GuildId, channel: ChannelId) -> Option<RoomHandle> {
        self.room(Chan { guild, channel })
    }

    pub fn set_room(&self, chan: Chan, h: Option<RoomHandle>) {
        let mut rooms = write(&self.rooms);
        match h {
            Some(h) => {
                rooms.insert(chan, h);
            }
            None => {
                rooms.remove(&chan);
            }
        }
    }

    /// Who is in which call (a snapshot).
    pub fn voice(&self) -> Arc<pb_policy::VoiceWorld> {
        self.voice.get()
    }

    pub fn update_voice<R>(&self, f: impl FnOnce(&mut pb_policy::VoiceWorld) -> R) -> R {
        self.voice.update(f)
    }

    pub fn is_listening(&self, guild: GuildId, user: UserId) -> bool {
        self.listening.lock().is_ok_and(|l| l.contains(&(guild, user)))
    }

    pub fn set_listening(&self, guild: GuildId, user: UserId, on: bool) {
        if let Ok(mut l) = self.listening.lock() {
            if on {
                l.insert((guild, user));
            } else {
                l.remove(&(guild, user));
            }
        }
        self.mark_person(guild, user);
    }

    /// The community's live views need rebuilding (its page and its sidebar entry).
    pub fn mark_guild(&self, guild: GuildId) {
        if let Ok(mut d) = self.dirty.lock() {
            d.guilds.insert(guild);
        }
    }

    /// A person's live views need rebuilding (their page, their community).
    pub fn mark_person(&self, guild: GuildId, user: UserId) {
        if let Ok(mut d) = self.dirty.lock() {
            d.people.insert((guild, user));
            d.guilds.insert(guild);
        }
    }

    pub fn mark_all(&self) {
        if let Ok(mut d) = self.dirty.lock() {
            d.all = true;
        }
    }

    pub fn jar(&self, guild: GuildId, user: UserId) -> u64 {
        self.jar
            .lock()
            .map(|j| j.get(&(guild, user)).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    pub fn clip(&self, h: &BlobHash) -> Option<ClipRecord> {
        read(&self.clips).get(h).cloned()
    }

    pub fn put_clip(&self, c: ClipRecord) {
        write(&self.clips).insert(c.render, c);
    }

    pub fn remove_clip(&self, h: &BlobHash) {
        write(&self.clips).remove(h);
        lock(&self.clip_pcm).remove(h);
    }

    pub fn next_sentence_no(&self, guild: GuildId, user: UserId) -> u32 {
        let mut m = self
            .sentence_no
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = m.entry((guild, user)).or_insert(0);
        *n += 1;
        *n
    }

    /// The bot owner (the application's owner).
    pub fn owner(&self) -> Option<UserId> {
        self.ctl().and_then(|c| c.me().owner)
    }

    pub fn bot(&self) -> Option<UserId> {
        self.ctl().map(|c| c.me().user.id)
    }

    /// Appends events (logged when the log refuses; moderation goes on in memory).
    pub async fn record(&self, events: Vec<Event>) -> bool {
        let new: Vec<NewEvent> = events.iter().filter_map(|e| e.to_new(None)).collect();
        if new.is_empty() {
            return true;
        }
        match self.deps.log.append(new).await {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(error = %e, "events could not be recorded");
                false
            }
        }
    }
}
