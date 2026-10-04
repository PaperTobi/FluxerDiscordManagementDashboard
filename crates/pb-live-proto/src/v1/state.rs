//! Topic states and their changes. The bot keeps each topic's latest state by applying the same deltas it sends, so
//! a snapshot is always exactly "everything up to `rev`".

use std::collections::VecDeque;

use pb_domain::{ActionKind, ActionOutcome, Audience, ChannelId, ClfLang, GuildId, Label, SentenceId, UserId};
use serde::{Deserialize, Serialize};

use super::conveyor::Stamps;
use super::wire::{ServerMs, Topic};

/// How much level history a live view keeps (the sparkline's width).
pub const LEVEL_WINDOW_MS: i64 = 12_000;
/// Finished sentences a person's live view keeps for the conveyor and the selected-sentence detail (all of them are
/// in History).
pub const LIVE_SENTENCES: usize = 50;
/// Activity items (warnings played, actions) a person's live view keeps (all of them are in History and Audit).
pub const LIVE_ACTIVITY: usize = 20;
/// Violations the wall and a community's overview list (all of them are in Reports).
pub const LIVE_VIOLATIONS: usize = 50;

/// A named reference to a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRef {
    pub id: ChannelId,
    pub name: String,
}

/// A person as shown: name and avatar URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Who {
    pub user: UserId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

// ------------------------------------------------------------------------------------------------ levels

/// One voice-activity frame: speech probability in percent and loudness in dBFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelFrame(pub u8, pub i8);

impl LevelFrame {
    pub fn new(probability: f32, dbfs: f32) -> LevelFrame {
        let p = (probability.clamp(0.0, 1.0) * 100.0).round() as u8;
        let db = dbfs.clamp(-127.0, 0.0).round() as i8;
        LevelFrame(p, db)
    }

    pub fn probability(self) -> f32 {
        f32::from(self.0) / 100.0
    }

    pub fn dbfs(self) -> f32 {
        f32::from(self.1)
    }
}

/// Contiguous frames ending at `end_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelRun {
    pub end_ms: ServerMs,
    pub frame_ms: u16,
    pub frames: Vec<LevelFrame>,
}

impl LevelRun {
    pub fn start_ms(&self) -> ServerMs {
        self.end_ms - i64::from(self.frame_ms) * self.frames.len() as i64
    }
}

/// The recent levels of one person (runs, oldest first; gaps are silence the bot did not receive).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Levels {
    pub runs: VecDeque<LevelRun>,
}

impl Levels {
    pub fn push(&mut self, run: LevelRun) {
        if run.frames.is_empty() {
            return;
        }
        match self.runs.back_mut() {
            Some(last) if last.end_ms == run.start_ms() && last.frame_ms == run.frame_ms => {
                last.frames.extend_from_slice(&run.frames);
                last.end_ms = run.end_ms;
            }
            _ => self.runs.push_back(run),
        }
        let end = self.end_ms().unwrap_or_default();
        let horizon = end - LEVEL_WINDOW_MS;
        while self.runs.front().is_some_and(|r| r.end_ms <= horizon) {
            self.runs.pop_front();
        }
        if let Some(first) = self.runs.front_mut() {
            let fm = i64::from(first.frame_ms.max(1));
            let excess = (horizon - first.start_ms()) / fm;
            if excess > 0 {
                first.frames.drain(..(excess as usize).min(first.frames.len()));
            }
        }
    }

    pub fn end_ms(&self) -> Option<ServerMs> {
        self.runs.back().map(|r| r.end_ms)
    }

    /// Voiced (probability ≥ 0.5) within `within_ms` before `now`.
    pub fn speaking(&self, now: ServerMs, within_ms: i64) -> bool {
        let Some(last) = self.runs.back() else { return false };
        let fm = i64::from(last.frame_ms);
        last.frames.iter().rev().enumerate().any(|(i, f)| {
            let t = last.end_ms - fm * i as i64;
            f.0 >= 50 && now - t < within_ms
        })
    }
}

// ------------------------------------------------------------------------------------------------ sentences

/// Why the segmenter cut a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutWhy {
    Pause,
    MaxLength,
    Muted,
    Left,
    StreamStalled,
    Shutdown,
}

/// Why a sentence was not scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropWhy {
    TooLittleSpeech,
    /// It overlapped the bot's own playback (its echo would be scored).
    OwnPlayback,
}

/// The model's verdict on a sentence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictView {
    /// Score per detection type, in model order.
    pub scores: [f32; 8],
    /// The bar each enabled type had to reach.
    pub thresholds: Vec<(Label, f32)>,
    pub flagged: Vec<Label>,
    pub language: ClfLang,
    pub infer_ms: u32,
    /// From the cut to the verdict.
    pub cut_to_verdict_ms: u32,
}

impl VerdictView {
    pub fn score(&self, label: Label) -> f32 {
        self.scores[label.index()]
    }

    /// The highest-scoring type.
    pub fn top(&self) -> Label {
        let mut best = (Label::Profanity, f32::MIN);
        for (i, l) in Label::ALL.iter().enumerate() {
            if self.scores[i] > best.1 {
                best = (*l, self.scores[i]);
            }
        }
        best.0
    }
}

/// What the bot did with a scored sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionView {
    NothingFlagged,
    InvalidScore,
    NoLongerTracked,
    Strike { strike: u32, of: u32 },
    Warn { step: u32, count: u32 },
    Observe { step: u32, count: u32 },
    Late { step: u32, count: u32 },
}

impl DecisionView {
    pub fn is_violation(self) -> bool {
        matches!(
            self,
            DecisionView::Warn { .. } | DecisionView::Observe { .. } | DecisionView::Late { .. }
        )
    }
}

/// One sentence on its way through the pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentenceCard {
    pub id: SentenceId,
    /// Running number per person since the bot started listening to them.
    pub no: u32,
    pub stamps: Stamps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<CutWhy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<DropWhy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionView>,
}

impl SentenceCard {
    /// No more changes will come (decided, dropped or failed).
    pub fn finished(&self) -> bool {
        self.stamps.decided.is_some() || self.stamps.dropped.is_some() || self.stamps.failed.is_some()
    }
}

/// Upserts `card` into `list` (ordered by `no`), keeping every unfinished card and the newest `keep` finished ones.
fn upsert_sentence(list: &mut Vec<SentenceCard>, card: SentenceCard, keep: usize) {
    match list.iter_mut().find(|c| c.id == card.id) {
        Some(c) => *c = card,
        None => {
            let at = list.partition_point(|c| c.no <= card.no);
            list.insert(at, card);
        }
    }
    let finished = list.iter().filter(|c| c.finished()).count();
    if finished > keep {
        let mut drop = finished - keep;
        list.retain(|c| {
            if drop > 0 && c.finished() {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

// ------------------------------------------------------------------------------------------------ activity

/// Something that happened to a person besides sentences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Activity {
    Play {
        id: u64,
        kind: pb_domain::PlayPurpose,
        at_ms: ServerMs,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        audience: Audience,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ended_ms: Option<ServerMs>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<bool>,
    },
    Action {
        id: u64,
        kind: ActionKind,
        at_ms: ServerMs,
        /// How long it lasts, in seconds.
        secs: Option<u64>,
        result: ActionOutcome,
    },
}

impl Activity {
    pub fn id(&self) -> u64 {
        match self {
            Activity::Play { id, .. } | Activity::Action { id, .. } => *id,
        }
    }
}

fn upsert_activity(list: &mut VecDeque<Activity>, item: Activity) {
    match list.iter_mut().find(|a| a.id() == item.id()) {
        Some(a) => *a = item,
        None => list.push_back(item),
    }
    while list.len() > LIVE_ACTIVITY {
        list.pop_front();
    }
}

/// A violation in a feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViolationItem {
    pub sentence: SentenceId,
    pub guild: GuildId,
    pub community: String,
    pub channel: ChannelRef,
    pub who: Who,
    pub label: Label,
    pub score: f32,
    pub step: u32,
    pub count: u32,
    pub decision: DecisionView,
    pub at_ms: ServerMs,
}

fn push_violation(list: &mut VecDeque<ViolationItem>, v: ViolationItem) {
    if list.iter().any(|x| x.sentence == v.sentence) {
        return;
    }
    list.push_front(v);
    while list.len() > LIVE_VIOLATIONS {
        list.pop_back();
    }
}

// ------------------------------------------------------------------------------------------------ sidebar

/// A person's live dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dot {
    /// Not in a voice channel.
    #[default]
    Away,
    /// In a call the bot is not listening to.
    InCall,
    Listening,
    Speaking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarPerson {
    pub who: Who,
    pub dot: Dot,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarCommunity {
    pub id: GuildId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub available: bool,
    pub paused: bool,
    pub people: Vec<SidebarPerson>,
}

/// The bot's connection to Fluxer, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FluxerState {
    /// No bot token yet (the setup wizard).
    #[default]
    NoToken,
    Connecting,
    Ready {
        bot: Who,
    },
    Reconnecting {
        error: String,
    },
    /// The instance has voice turned off.
    NoVoice,
    /// Fluxer rejected the token; it has to be replaced.
    TokenRejected,
    /// The gateway refused for good (it needs an update or another setup).
    Stopped {
        error: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarState {
    pub communities: Vec<SidebarCommunity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SidebarDelta {
    /// Adds or replaces a community (with its people).
    Community {
        community: SidebarCommunity,
    },
    CommunityGone {
        guild: GuildId,
    },
    Dot {
        guild: GuildId,
        user: UserId,
        dot: Dot,
    },
}

impl SidebarState {
    pub fn apply(&mut self, d: &SidebarDelta) {
        match d {
            SidebarDelta::Community { community } => {
                match self.communities.iter_mut().find(|c| c.id == community.id) {
                    Some(c) => *c = community.clone(),
                    None => self.communities.push(community.clone()),
                }
                self.communities
                    .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
            }
            SidebarDelta::CommunityGone { guild } => self.communities.retain(|c| c.id != *guild),
            SidebarDelta::Dot { guild, user, dot } => {
                if let Some(p) = self
                    .communities
                    .iter_mut()
                    .find(|c| c.id == *guild)
                    .and_then(|c| c.people.iter_mut().find(|p| p.who.user == *user))
                {
                    p.dot = *dot;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ wall

/// A person the bot hears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub guild: GuildId,
    pub community: String,
    pub who: Who,
    pub channel: ChannelRef,
    #[serde(default)]
    pub levels: Levels,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<SentenceCard>,
    /// The last flagged-or-not verdict's cut-to-verdict time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_ms: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WallState {
    pub tiles: Vec<Tile>,
    pub violations: VecDeque<ViolationItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WallDelta {
    /// Adds a tile or updates its names and channel (levels and the latest sentence are kept).
    Tile {
        tile: Box<Tile>,
    },
    TileGone {
        guild: GuildId,
        user: UserId,
    },
    Levels {
        guild: GuildId,
        user: UserId,
        run: LevelRun,
    },
    Sentence {
        guild: GuildId,
        user: UserId,
        card: Box<SentenceCard>,
    },
    Violation {
        violation: ViolationItem,
    },
}

impl WallState {
    fn tile(&mut self, guild: GuildId, user: UserId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|t| t.guild == guild && t.who.user == user)
    }

    pub fn apply(&mut self, d: &WallDelta) {
        match d {
            WallDelta::Tile { tile } => match self.tile(tile.guild, tile.who.user) {
                Some(t) => {
                    t.community.clone_from(&tile.community);
                    t.who = tile.who.clone();
                    t.channel = tile.channel.clone();
                }
                None => {
                    self.tiles.push((**tile).clone());
                    self.tiles.sort_by(|a, b| {
                        (a.community.to_lowercase(), a.who.name.to_lowercase(), a.who.user).cmp(&(
                            b.community.to_lowercase(),
                            b.who.name.to_lowercase(),
                            b.who.user,
                        ))
                    });
                }
            },
            WallDelta::TileGone { guild, user } => self.tiles.retain(|t| !(t.guild == *guild && t.who.user == *user)),
            WallDelta::Levels { guild, user, run } => {
                if let Some(t) = self.tile(*guild, *user) {
                    t.levels.push(run.clone());
                }
            }
            WallDelta::Sentence { guild, user, card } => {
                if let Some(t) = self.tile(*guild, *user)
                    && t.latest.as_ref().is_none_or(|l| l.id == card.id || l.no <= card.no)
                {
                    if let Some(v) = &card.verdict {
                        t.lag_ms = Some(v.cut_to_verdict_ms);
                    }
                    t.latest = Some((**card).clone());
                }
            }
            WallDelta::Violation { violation } => push_violation(&mut self.violations, violation.clone()),
        }
    }
}

// ------------------------------------------------------------------------------------------------ guild

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    pub who: Who,
    pub bot: bool,
    pub tracked: bool,
    pub active: bool,
    pub muted: bool,
    pub deaf: bool,
    pub listening: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub channel: ChannelRef,
    pub participants: Vec<Participant>,
    pub bot_in: bool,
    pub can_speak: bool,
    pub encrypted: bool,
    pub audience: Audience,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedPerson {
    pub who: Who,
    pub active: bool,
    pub paused: bool,
    /// Tracked in every community (global setting).
    pub everywhere: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelId>,
}

/// How far the bot is with a voice channel it follows someone into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotJoin {
    /// About to join (waiting a moment, in case the person moves on at once).
    Waiting,
    /// Joining and connecting to the call.
    Joining,
    /// In the call.
    Connected,
    Leaving,
    /// Joining failed; trying again after a pause.
    Retrying,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub channel: ChannelId,
    pub state: BotJoin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuildState {
    pub id: GuildId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub available: bool,
    pub allowed: bool,
    pub paused: bool,
    pub calls: Vec<Call>,
    pub tracked: Vec<TrackedPerson>,
    pub connections: Vec<Connection>,
    pub violations: VecDeque<ViolationItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GuildDelta {
    /// Everything but the violations (the community view is small and rebuilt as a whole).
    View {
        view: Box<GuildState>,
    },
    Violation {
        violation: ViolationItem,
    },
}

impl GuildState {
    pub fn apply(&mut self, d: &GuildDelta) {
        match d {
            GuildDelta::View { view } => {
                let violations = std::mem::take(&mut self.violations);
                *self = (**view).clone();
                self.violations = violations;
            }
            GuildDelta::Violation { violation } => push_violation(&mut self.violations, violation.clone()),
        }
    }
}

// ------------------------------------------------------------------------------------------------ person

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelRef>,
    pub muted: bool,
    pub deaf: bool,
    /// The bot receives this person's microphone.
    pub listening: bool,
    pub bot_in_call: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracking {
    /// On this community's list or tracked everywhere.
    pub listed: bool,
    /// Listed and not paused.
    pub active: bool,
    pub everywhere: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub jar: u64,
    /// Violations in the escalation window.
    pub in_window: u32,
    /// The escalation window; `None` = unlimited.
    pub window_ms: Option<u64>,
    pub today: u32,
    /// The escalation step the next violation is on (0 = none yet).
    pub step: u32,
}

/// The settings that shape the live view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonSummary {
    pub observe_only: bool,
    pub audience: Audience,
    pub strikes: u32,
    pub thresholds: Vec<(Label, f32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonState {
    pub guild: GuildId,
    pub who: Who,
    pub presence: Presence,
    pub tracking: Tracking,
    pub counts: Counts,
    pub summary: PersonSummary,
    #[serde(default)]
    pub levels: Levels,
    pub sentences: Vec<SentenceCard>,
    pub activity: VecDeque<Activity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PersonDelta {
    Who { who: Who },
    Presence { presence: Presence },
    Tracking { tracking: Tracking },
    Counts { counts: Counts },
    Summary { summary: PersonSummary },
    Levels { run: LevelRun },
    Sentence { card: Box<SentenceCard> },
    Activity { item: Activity },
}

impl PersonState {
    pub fn apply(&mut self, d: &PersonDelta) {
        match d {
            PersonDelta::Who { who } => self.who = who.clone(),
            PersonDelta::Presence { presence } => self.presence = presence.clone(),
            PersonDelta::Tracking { tracking } => self.tracking = tracking.clone(),
            PersonDelta::Counts { counts } => self.counts = counts.clone(),
            PersonDelta::Summary { summary } => self.summary = summary.clone(),
            PersonDelta::Levels { run } => self.levels.push(run.clone()),
            PersonDelta::Sentence { card } => upsert_sentence(&mut self.sentences, (**card).clone(), LIVE_SENTENCES),
            PersonDelta::Activity { item } => upsert_activity(&mut self.activity, item.clone()),
        }
    }
}

// ------------------------------------------------------------------------------------------------ system

/// A model the bot runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Classifier,
    VoiceActivity,
    Speech,
}

/// What is wrong with a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProblem {
    /// Its thread ended or hangs (restart the bot).
    NotAnswering,
    /// Text-to-speech without any voice.
    NoVoices,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStatus {
    pub model: Model,
    /// Where it runs (cpu with its threads, or the GPU's name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ModelProblem>,
}

/// A queue of model work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Queue {
    /// Sentences waiting to be scored.
    Scoring,
    /// Speech waiting to be made.
    Speech,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueStatus {
    pub queue: Queue,
    pub waiting: u64,
    pub done: u64,
    /// How long the oldest waiting job has waited.
    pub oldest_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStatus {
    pub log_bytes: u64,
    pub blob_bytes: u64,
    pub free_bytes: u64,
    /// Events the search index has not caught up with.
    pub index_behind: u64,
    /// What keeps the search index from catching up (it keeps trying).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_problem: Option<String>,
    /// Events the search index could not read (from a newer version of the bot) and left out.
    #[serde(default)]
    pub index_skipped: u64,
    /// The event log stopped writing (why); nothing is written until an owner asks to try again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_halted: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemState {
    pub version: String,
    pub started_ms: ServerMs,
    pub fluxer: FluxerState,
    pub models: Vec<ModelStatus>,
    pub queues: Vec<QueueStatus>,
    pub storage: StorageStatus,
    pub rooms: u32,
    pub streams: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SystemDelta {
    /// The whole status (sent at most a few times a second).
    Status { status: Box<SystemState> },
}

impl SystemState {
    pub fn apply(&mut self, d: &SystemDelta) {
        match d {
            SystemDelta::Status { status } => *self = (**status).clone(),
        }
    }
}

// ------------------------------------------------------------------------------------------------ any topic

/// The state of any topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopicState {
    Sidebar(SidebarState),
    Wall(WallState),
    Guild(Box<GuildState>),
    Person(Box<PersonState>),
    System(Box<SystemState>),
}

/// A change to any topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopicDelta {
    Sidebar(SidebarDelta),
    Wall(WallDelta),
    Guild(GuildDelta),
    Person(PersonDelta),
    System(SystemDelta),
}

/// A delta for another kind of topic than the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a delta for another kind of topic")]
pub struct KindMismatch;

impl TopicState {
    pub fn apply(&mut self, d: &TopicDelta) -> Result<(), KindMismatch> {
        match (self, d) {
            (TopicState::Sidebar(s), TopicDelta::Sidebar(d)) => s.apply(d),
            (TopicState::Wall(s), TopicDelta::Wall(d)) => s.apply(d),
            (TopicState::Guild(s), TopicDelta::Guild(d)) => s.apply(d),
            (TopicState::Person(s), TopicDelta::Person(d)) => s.apply(d),
            (TopicState::System(s), TopicDelta::System(d)) => s.apply(d),
            _ => return Err(KindMismatch),
        }
        Ok(())
    }

    /// Whether this state belongs to `topic`.
    pub fn fits(&self, topic: &Topic) -> bool {
        match (self, topic) {
            (TopicState::Sidebar(_), Topic::Sidebar)
            | (TopicState::Wall(_), Topic::Wall)
            | (TopicState::System(_), Topic::System) => true,
            (TopicState::Guild(g), Topic::Guild { guild }) => g.id == *guild,
            (TopicState::Person(p), Topic::Person { guild, user }) => p.guild == *guild && p.who.user == *user,
            _ => false,
        }
    }
}
