# 0004 — Engine: actors that own their state, supervision, a pipeline without disk waits, orderly shutdown

Date: 2026-10-04. Status: decided; being implemented step by step (section 3). The engine was first ported from the
Python bot's shared controller: one `Core` with ~20 lock-guarded fields, actors awaiting fsyncs in their loops,
unsupervised tasks and no shutdown order. This is its rebuild.

Paths without a directory are in `crates/pb-engine/src/v1/`.

## 1. Today's state

### 1.1 The `Core` fields (`core.rs`), with who writes and reads each

| # | Field and type | Writers | Readers |
|---|---|---|---|
| 1 | `deps: Deps` | none (fixed at start) | everything |
| 2 | `settings: SettingsService` (`RwLock<Arc<SettingsTree>>` + `watch<u64>` + a tokio `Mutex<()>` for writes) | `SettingsService::change`/`reload`, called from `people::track`/`untrack`, `commands::run`, `Engine::reload`, and web forms/voice/setup | about 40 `current()` sites in library, room, actions, cells, commands, control, engine, speak, reports, track, moderation, people. `watch()` in `control::run`, `engine::supervise`, `follow_threads`, `track::run`, `room::run` |
| 3 | `guilds: Snapshot<Guilds>` (`RwLock<Arc<Guilds>>` plus `make_mut`, which deep-copies all guilds whenever a reader holds a snapshot) | `control::Session::event` (`gs.apply`, `gs.bot`), `control::remember_community`, `Engine::start` (seeding), `people::learn_person` | cells (`who`, `sidebar_community`, `communities`, `guild_state`, `presence`, `Cells::ensure`, `refresh`), `control::step`, `commands` (`presence_text`, `run`), `Engine::{guilds, bot_permissions, admin_guilds}`, `moderation::decide`, `reports::{owner_dm, digest_once}`, `speak::fields_for`, `room::on_subscribed` |
| 4 | `ctl: RwLock<Option<Arc<dyn FluxerCtl>>>` | `engine::supervise` through `set_ctl` (3 sites) | `commands::on_message`, `actions::{step_action, undo}`, `people::learn_person`, `Engine::{redirect_registered, identity, endpoints, admin_guilds, search_members}`, `system_status`, `reports::send`, `room::no_speak`. Through `owner()`/`bot()`/`avatar_url()`: commands, `is_owner`, reports, `room::wanted`, `people::track`, `cells::who`, moderation |
| 5 | `live: Live` (the pb-live Hub, which has its own Mutex) | actions, cells, control (dots), moderation, room, playback, track, system_status | the web |
| 6 | `clips: RwLock<BTreeMap<BlobHash, ClipRecord>>` | `Engine::start`, `add_clip`, `update_clip`, `remove_clip` | `speak::clip_info`, `speak::render` (transcripts), `Engine::{clip, update_clip, remove_clip}` |
| 7 | `rooms: RwLock<BTreeMap<Chan, RoomHandle>>` | `control::run` (RoomDown, session end), `Session::exec` (Connect, Close) | control (VoiceState dot, Close), `commands::status`, cells (`guild_state`, `presence`), `Engine::{rooms, say}`, `actions::undo`, `system_status` |
| 8 | `sentence_no: Mutex<HashMap<(GuildId, UserId), u32>>` | `track::handle` (Open, Cut) | same |
| 9 | `speech: Mutex<AudioCache<SpeechKey>>` | `speak::speech`, `Engine::reload` (clear) | `speak::speech` |
| 10 | `clip_pcm: Mutex<AudioCache<BlobHash>>` | `speak::clip_pcm`, `Core::remove_clip` | `speak::clip_pcm` |
| 11 | `moderation: UnboundedSender<ModMsg>` | sent to by track (Heard), `Engine::start` (Seed), `commands::reset_jar` (JarReset), `cells::fill_person` (Counts) | moderation actor |
| 12 | `voice: Snapshot<VoiceWorld>` | `control::Session::event` (5 sites) | `control::step`, cells (`dot`, `guild_state`, `presence`, `refresh`), actions (`already_muted`, undo room lookup), `commands::status`, `Engine::{voice, say_to}` |
| 13 | `jar: Mutex<HashMap<(GuildId, UserId), u64>>` | `Engine::start`, `moderation::run` (reset), `moderation::decide` (+1) | `moderation::counts`, `cells::person_state`, `commands` (Jar), `reports::digest_once` |
| 14 | `undo: UnboundedSender<ActionRecord>` | `actions::step_action` | `undo_scheduler` |
| 15 | `no_repeat: Mutex<NoRepeat>` | `speak::render`, called from playback, preview **and prerender** | same |
| 16 | `listening: Mutex<BTreeSet<(GuildId, UserId)>>` | room (TrackUnpublished, end, reconcile, `on_subscribed`) through `set_listening` | cells (`dot`, `guild_state`, `presence`, `refresh` wall) |
| 17 | `names_recorded: Mutex<BTreeMap<(UserId, Option<GuildId>), Names>>` (new in the tree) | `Engine::start`, `control::remember_person` | `control::remember_person` |
| 18 | `speaking: Mutex<BTreeSet<Chan>>` | `room::run` | `cells::guild_state` |
| 19 | `conns: RwLock<BTreeMap<Chan, BotJoin>>` | `control::step` | `cells::guild_state` |
| 20 | `dirty: Mutex<Dirty>` | `mark_*` from control, room, people, `set_listening`, `Engine::start` | `cells::refresh` (takes it) |
| 21 | `connection: watch::Sender<Login>` | `engine::supervise`, control (Ready, Resumed, Down) | `Engine::{connection, login_outcome}`, `system_status` |
| 22 | `redirects: Mutex<(Option<Instant>, Vec<String>)>` | `set_ctl`, `redirect_registered` | `redirect_registered` |

Two more shared locks sit inside each room: `participants: Arc<Mutex<BTreeMap<Identity, Participant>>>` (room writes, playback reads) and `echo: Arc<Mutex<EchoGuard>>` (playback writes, tracks read).

Lock poisoning is handled three different ways: `unwrap_or_else(into_inner)` in `read`/`write`/`lock`, silent skips with `if let Ok`/`.ok()` (for example `mark_*`, `jar`, `set_listening`), and `.lock().ok()?`.

### 1.2 Every spawn site

**In the engine:**
- `engine.rs` `Engine::start`: `moderation::run`, `actions::undo_scheduler`, `reports::digest_scheduler`, `follow_threads`, `cells::refresh`, `supervise` (the only JoinHandle kept) and `system_status`. If one of these panics or ends, nothing notices. For example, Heard messages would pile up forever in the moderation channel.
- `control.rs`: `commands::on_message` per chat message; one voice-state op task per `Action::VoiceState` (leaves retried 6 times, 2 s apart); `record_names` per batch (working tree).
- `room.rs`: `room::spawn`→`run`; `playback` (aborted at the end); `unsubscribe` twice; `subscribe`; `track::run`; `speak::prerender`.
- `track.rs`: one classify-and-send task per sentence.
- `moderation.rs`: one `step_action` + `reports::flagged` task per flagged sentence.
- `cells.rs`: `fill_person` per new person cell.
- `library.rs`: `spawn_blocking` for decoding (awaited, fine).

**Outside the engine (they matter for shutdown):**
- pb-store: the log writer thread (`log.rs:197`), scan/verify `spawn_blocking`, the index follower `tokio::spawn` (`index/mod.rs:92`, not supervised), the index db thread (`db.rs:134`), blob `spawn_blocking`.
- pb-infer model threads (`mod.rs:482`); pb-fluxer gateway task (`gateway.rs:195`); pb-voice-livekit (`lib.rs:77`, `:251`).
- pb-web-server (`server.rs:311`); `crates/pb/src/run.rs:223`.

### 1.3 Disk waits on hot paths
1. **`moderation::decide`** awaits `blobs.put(wav)` (fsync), then `core.record(events)` (fsync), and only then calls `room.play`. Every warning waits for two fsyncs, and the next decision waits too. The WAV is also built for every sentence, even when it is neither kept nor attached.
2. **`room::playback`** awaits `core.record(Played)` after each item. The next queued warning and the `say()` reply both wait for the fsync.
3. **`actions::step_action`** awaits the Action record before the announcement and the report. **`reports::send`** awaits each MessageSent record.
4. **Smaller ones:** `people::learn_person` and `commands::reset_jar` await their records. `speak::clip_pcm` decodes and resamples on a runtime thread. `room::reconcile` awaits `set_audience` (a network call) inside the room loop.

### 1.4 Other defects found while reading
- **Shutdown order is broken.** `supervise` calls `ctl.close()` first. Only then does `control::run` see the event channel close and run `machine.shutdown()`, so its Leave ops go to a closed gateway, in spawned tasks nobody waits for. Rooms get `Close` but nobody waits for them. Tracks are not flushed with `CutCause::Shutdown`. `Stopped` can be recorded before in-flight Played/Action events. `inference.shutdown()` blocks a runtime worker (`crates/pb/src/run.rs`).
- **Voice-state ops can reorder.** One spawned task per op means a Join and a Leave can race.
- **Name records can reorder** (working tree). `record_names` spawns one task per batch, so two PersonSeen events for the same person can land out of order and leave a stale name in the index.
- **Jar reset race.** `reset_jar` records JarReset, then sends `ModMsg::JarReset`. A Sentence with `jar: true` decided in between ends up as 0 in memory but 1 in the index.
- **Prerender keys never match live keys.** Prerender uses `Count = step` (it should be the window count), `channel = None` (live has the channel) and `heard = None` (the language differs with `auto`). It also calls `render()`, which writes the no-repeat memory, so prerendering changes which clip plays live.
- **`AudioCache::insert`:** an entry larger than the whole budget evicts every other entry.
- **Per-person decision order** depends on the order in which tokio wakes the per-sentence tasks.
- **Two writers for the audience:** `room::reconcile` and playback narrowing race on `set_audience`.

---

## 2. Target design

### 2.1 Principles
- **`Core` goes away.** An immutable `Arc<Ctx>` takes its place: handles (mailbox senders), watch receivers and services. Nothing in `Ctx` is behind our own lock.
- **Leaf Mutexes are allowed only where justified:** speech/clip LRU, the in-flight maps, `NoRepeat`, the redirect memo. Each is a short synchronous critical section with no await and no I/O.
- **Every task is spawned through the `Supervisor`.** It uses a tokio-util `TaskTracker` and a `CancellationToken` tree. tokio-util 0.7.19 is already in Cargo.lock; add it to the workspace with `features = ["rt"]`, plus `futures` for `catch_unwind`.

```rust
// ctx.rs (replaces core.rs)
pub(crate) struct Ctx {
    pub deps: Deps, pub world: World, pub live: Live,
    pub recorder: RecorderHandle, pub settings: SettingsService, pub directory: DirectoryHandle,
    pub gateway: GatewayHandle, pub moderation: ModerationHandle, pub enforcer: EnforcerHandle,
    pub undo: Addr<UndoMsg>, pub digest: Addr<DigestMsg>, pub library: LibraryHandle,
    pub views: Addr<ViewsMsg>, pub speech: SpeechService, pub sup: Supervisor,
    pub phase: watch::Receiver<Phase>,
}
#[derive(Clone)] pub(crate) struct World {
    pub settings: watch::Receiver<Arc<SettingsView>>, pub session: watch::Receiver<Arc<SessionView>>,
    pub guilds: watch::Receiver<Arc<Guilds>>, pub voice: watch::Receiver<Arc<VoiceWorld>>,
    pub rooms: watch::Receiver<Arc<RoomsView>>, pub library: watch::Receiver<Arc<ClipLibrary>>,
}   // accessors: settings()/session()/guilds()/voice()/rooms()/library() -> Arc<_> (borrow().clone())
```

### 2.2 Mailboxes and the supervisor (`mailbox.rs`, `supervise.rs`, `health.rs`)

```rust
pub(crate) fn mailbox<T>(name: &'static str) -> (Addr<T>, Mailbox<T>);       // unbounded (no caps)
#[derive(Clone)] pub(crate) struct Addr<T> { tx: mpsc::UnboundedSender<T>, stats: Arc<MailboxStats> }
impl<T> Addr<T> { fn send(&self, m: T) -> Result<(), Closed>;
                  async fn ask<R>(&self, f: impl FnOnce(oneshot::Sender<R>) -> T) -> Result<R, Closed>; }
pub(crate) struct Mailbox<T> { rx: mpsc::UnboundedReceiver<T>, stats: Arc<MailboxStats> } // recv(), try_recv()
struct MailboxStats { queued: AtomicU64, last_recv_ms: AtomicU64 }

pub(crate) trait Actor: Sized + Send + 'static {
    type Msg: Send + 'static;
    const NAME: &'static str;
    const POLICY: Policy;
    /// First start and every restart: rebuild state (from the index/log where needed).
    fn start(ctx: &Arc<Ctx>, attempt: u32) -> impl Future<Output = Result<Self, StoreError>> + Send;
    fn run(self, ctx: Arc<Ctx>, mb: &mut Mailbox<Self::Msg>, life: Life)
        -> impl Future<Output = Result<(), ActorError>> + Send;
}
pub(crate) struct Life { pub cancel: CancellationToken, pub beat: Beat, pub phase: watch::Receiver<Phase> }
pub(crate) enum Policy { Restart(Backoff), Fatal }
pub(crate) struct Backoff { first: Duration /*100ms*/, max: Duration /*30s*/, healthy_after: Duration /*60s*/,
                            escalate_after: u32 /*5*/, within: Duration /*10min*/ }
pub(crate) struct Supervisor { tracker: TaskTracker, root: CancellationToken,
                               health: watch::Sender<Arc<HealthView>>, fatal: watch::Sender<Option<FatalError>> }
impl Supervisor { fn spawn<A: Actor>(&self, ctx: Arc<Ctx>, mb: Mailbox<A::Msg>) -> ActorRef;
                  fn spawn_task(&self, name: &'static str, fut: impl Future<Output=()> + Send + 'static); // short tasks
                  fn fatal(&self, e: FatalError); }
```

How the wrapper works:
- **It keeps the actor alive across crashes.** One wrapper task per actor runs `start`, then `AssertUnwindSafe(actor.run(.., &mut mailbox, ..)).catch_unwind()`. The **mailbox belongs to the wrapper and outlives a crash**; only the message being processed is lost. A panic or an `Err` restarts with backoff. More than 5 crashes within 10 minutes escalates to fatal. `Policy::Fatal` (and escalation) calls `sup.fatal(..)`. Supervision needs `panic = "unwind"`, which is the current default.
- **Every actor reports liveness.** Each actor calls `life.beat.beat()` per message and on a 5 s idle tick.
- **Health is public.** New types in `health.rs`:
  - `pub struct EngineHealth { actors: Vec<ActorHealth>, rooms: u32, tracks: u32, stuck_model: Option<pb_infer::Worker>, fatal: Option<FatalError> }`
  - `pub struct ActorHealth { name: String, state: ActorState, restarts: u32, since_beat_ms: Option<u64>, queued: u64, last_error: Option<String> }`
  - `pub enum ActorState { Starting, Running, Restarting { attempt: u32 }, Stopped, Failed, NotAnswering }`. NotAnswering means queued > 0 and no beat for 60 s.
- **New Engine API:** `pub fn health(&self) -> EngineHealth` and `pub async fn fatal(&self) -> FatalError`.

### 2.3 Actors: state, messages, handles, restart policy

| Actor | Owns | Messages | Handle | Policy and rebuild |
|---|---|---|---|---|
| **Recorder** (`recorder.rs`), the single log writer | FIFO `VecDeque<Pending>`; `committed: watch::Sender<u64>` | `RecordMsg::{Events{events: Vec<NewEvent>, ack: Option<oneshot<Result<Vec<EventRef>,StoreError>>>}, Sentence{record: Box<SentenceRecord>, evidence: Option<Arc<Evidence>>, span}, Barrier(oneshot<Option<u64>>)}` | `RecorderHandle { record(&self, Vec<Event>); record_acked(&self, Vec<Event>) -> Ack /*enqueued at call time*/; sentence(..); async barrier() -> Option<u64>; committed() -> u64 }` | **Fatal** (queued events cannot be rebuilt). Pipelined: a Sentence with evidence gets its blob write as a tracked task; the ready prefix is committed in order as **one** `log.append`. On failure, acks get the error and fire-and-forget batches are logged at error (as today's `Core::record`) |
| **Settings** (`settings.rs`) | the current `Arc<SettingsView>`, inside the watch sender held by the wrapper | `SettingsMsg::{Change{by, edit: Box<dyn FnOnce(&mut SettingsTree)->Result<Vec<Change>,SettingError>+Send>, reply}, Reload{reply}}` | `SettingsService` (public; same API plus `view()`) | Restart; the state survives in the watch. Order kept: files write → publish view → `record_acked(SettingsChanged)` |
| **Directory** (`directory.rs`) | `Guilds` (mutable, `Arc` per guild), `names_recorded`, `known_communities` | `DirMsg::{Gateway(Arc<GatewayEvent>), Learned{guild, member: Member}, Seed{communities, people}, Sync(oneshot<Arc<Guilds>>)}` | `DirectoryHandle { send(..); async synced() -> Arc<Guilds> }` | Restart: reseed from `index.communities()/people()`, then `gateway.reconnect()` (roles, channels and members come back with Ready). Publishes once per drained batch. Changed names go to `recorder.record(PersonSeen/CommunitySeen)` in FIFO order |
| **Gateway** (`gateway.rs`, was `engine::supervise`) | backoff, the current session | `restart: watch<u64>`; `GatewayMsg::Shutdown(Step, oneshot<()>)` | `GatewayHandle { reconnect() -> u64; async login_outcome(..) }` | Restart with backoff. Publishes `watch<Arc<SessionView>>`. Runs Control as a child with `child_token` |
| **Control** (per session, `control.rs`) | `FollowMachine`, `VoiceWorld`, `Burst`, grants, session_id, presence, blocked, the room registry `BTreeMap<Chan, RoomEntry{handle, task}>` in a JoinSet, `VoiceStateSender` (ordered FIFO) | `ControlMsg::{RoomUp{chan, speaking: bool}, Listening{chan, user, on: bool}, RoomDown{chan, reason}, Step(ShutdownStep, oneshot<()>)}` | internal `Addr` | Its exit ends the session and the gateway logs in again; the follow machine rebuilds from Ready. Publishes `voice` and `rooms` watches once per loop iteration (coalesced). Commands run as tracked short tasks |
| **Room** (per channel, `room.rs`) | `Box<dyn VoiceRoom>` as `Arc`, participants (owned), `tracks: JoinSet` + `HashMap<TrackKey, TrackRun>`, subscribing, greeted | `RoomCmd::{Play(Box<PlayItem>), Flush(oneshot<()>), Drain(oneshot<()>), Close}` | `RoomHandle { chan, tx: Addr<RoomCmd> }; play()` | Its exit sends RoomDown; the follow machine backs off and rejoins (the existing logic). Publishes `watch<Arc<Participants>>` for playback |
| **Playback** (child of the room, `playback.rs`) | `AudioOut`, `echo: watch::Sender<EchoGuard>`, default audience; the **only writer of the audience** | `PlaybackMsg::{Play(Box<PlayItem>), DefaultAudience(AudienceSet), Drain(oneshot)}` | internal | Ends with the room |
| **Track** (per microphone, `track.rs`) | `VadStream`, `FrameAssembler`, `PcmRing`, `Segmenter`, open cards, `no: Arc<AtomicU32>` (from moderation) | `TrackCmd::{Muted(bool), Stop(FlushReason)}` | internal | Its exit removes it from `tracks`; the next reconcile subscribes again |
| **Scorer** (per track) | an ordered pipeline (`FuturesOrdered`) of `classify_job` calls | `mpsc<ScoreJob>`; sends `ModMsg::Heard` **in cut order** | none | Drains, then exits when the track closes its input |
| **Moderation** (`moderation.rs` + `decide.rs`) | `Decider`, `Violations`, `jar`, `counters: HashMap<(g,u), Arc<AtomicU32>>`, `last_heard: HashMap<(g,u), ClfLang>` | `ModMsg::{Heard(Box<Heard>), ResetJar{guild, user, by, ack}, Counts{g,u,reply}, Jars{reply}, Counter{g,u,reply}, Predict{g,u,reply: oneshot<Prediction>}, Barrier(oneshot<()>)}` | `ModerationHandle` | Restart: `recorder.barrier()` → `index.caught_up(seq)` → `violation_times` + `jar(None)`. Strike memory is lost, as on a process restart today |
| **Enforcer** (`enforcer.rs`) | per-person queues `HashMap<(g,u), VecDeque<Followup>>` and a `JoinSet` of running jobs | `EnforcerMsg::{Followup(Box<Followup>), Drain(oneshot<()>)}` | `EnforcerHandle` | Restart. A job that panics is recorded as `Action{outcome: Failed{"internal error"}}`, not dropped silently |
| **Undo** (`undo.rs`, out of `actions.rs`) | the due list | `UndoMsg::Schedule(ActionRecord)` | `Addr` | Restart: reseed from `index.pending_undos()` after barrier and catch-up |
| **Digest** (`digest.rs`, out of `reports.rs`) | none | `DigestMsg::SendNow(oneshot<Result<bool,EngineError>>)` plus a 60 s timer | `Addr` | Restart. After a send it awaits the `MessageSent` ack and `index.caught_up(seq)`, so a slot is never sent twice |
| **Library** (`library.rs`) | `ClipLibrary`, inside the watch sender | `LibraryMsg::Commit{events: Vec<Event>, change: LibraryChange::{Put(ClipRecord), Remove(BlobHash)}, reply}` | `LibraryHandle`. Decoding, blob puts and the self-check run in the **caller's** task; the actor only commits after the ack | Restart: reseed from `index.clips()` |
| **Views** (`views.rs`, was `cells::refresh`) | `sidebar_known`, `wall_tiles`, `dirty` | `ViewsMsg::{Mark(Mark::{Guild(g), Person(g,u), All}), FillPerson{g,u,reply: Option<oneshot<()>>}}` | `Addr` | Restart, then mark everything dirty |
| **System** (`system.rs`) | `started` | timer only | none | Restart. Also: a model thread that has *ended* (`inference.stuck()` with `is_finished`) triggers fatal |
| **Threads** (`follow_threads`) | the applied thread counts | settings watch | none | Restart |

**Services that are not tasks:**
- `SpeechService` (see 2.7).
- `Live` (the Hub).
- `Cells`, the `CellSource`, which builds from World snapshots and sends `FillPerson`.
- `RedirectMemo`, a leaf `Mutex` in `Engine`, keyed by session generation (no reset hook needed).

**Key internal types:**

```rust
pub(crate) struct ScoreJob { id: SentenceId, no: u32, chan: Chan, user: UserId, room: RoomHandle, started: Timestamp,
  dur_ms: u32, level_db: f32, cut: CutCause, cut_why: CutWhy, pcm: Arc<[f32]>, cut_mono: f64,
  deadline: Option<tokio::time::Instant>, stamps: Stamps, span: tracing::Span }
pub(crate) struct Heard { job: ScoreJob, scored: Result<Scored, InferError> }
pub(crate) struct Prediction { next_count: u32, next_step: Option<(u32, EscalationStep)>, next_strike: u32, heard: Option<ClfLang> }
pub(crate) struct Followup { sentence: Arc<SentenceRecord>, step: Option<EscalationStep>, room: RoomHandle,
  heard: ClfLang, evidence: Option<Arc<Evidence>>, span: Span }
pub(crate) struct PlayItem { /* existing fields */ span: Span }
```

### 2.4 The watch snapshots
- **`settings: Arc<SettingsView>`** (2.8).
- **`session: Arc<SessionView { attempt, state: Connection, ctl: Option<Arc<dyn FluxerCtl>>, me: Option<BotIdentity>, endpoints: Option<Endpoints> }>`.** This replaces `ctl`, `connection` and `Login`.
- **`guilds: Arc<Guilds>`.** Shape:
  - `Guilds { bot, guilds: BTreeMap<GuildId, Arc<GuildInfo>>, known: Arc<BTreeMap<..>>, known_communities: Arc<BTreeMap<..>> }`
  - `GuildInfo { name, icon, owner, available, roles: Arc<BTreeMap>, channels: Arc<BTreeMap>, people: Arc<BTreeMap> }`

  A publish clones only pointers, plus the touched map of the guild that changed (`Arc::make_mut`). There are no deep copies per event.
- **`voice: Arc<VoiceWorld>`**, owned by Control.
- **`rooms: Arc<RoomsView { rooms: BTreeMap<Chan, RoomView{handle, speaking, listening: BTreeSet<UserId>}>, joins: BTreeMap<Chan, BotJoin> }>`.** This replaces `rooms`, `conns`, `speaking` and `listening`.
- **`library: Arc<ClipLibrary>`.**
- **`committed: u64`** (Recorder), **`health: Arc<HealthView>`** and **`fatal: Option<FatalError>`** (Supervisor), **`phase: Phase`** (Engine).
- **Moderation publishes no watch.** Jar, counts and predictions are queries. `person_state` starts with `Counts::default()` and `FillPerson` fills it in. `Engine::person_view` already awaits the fill, so SSR still shows real numbers.

### 2.5 The supervision tree
```
Engine (root token, root TaskTracker)
├─ recorder [Fatal]                  token: own, cancelled only after DrainLog
├─ settings, directory*, library, moderation, enforcer, undo, digest, views, system, threads [Restart]
└─ gateway [Restart+backoff]          token: root.child
   └─ control (per session)           token: gateway.child → exit = session end → relogin
      ├─ voice-state sender (ordered), command tasks (tracked)
      └─ room (per channel)           token: session.child → exit = RoomDown → follow machine backoff
         ├─ playback
         ├─ subscribe/unsubscribe one-shots
         └─ track (per microphone)    token: room.child → exit = resubscribe on reconcile
            └─ scorer
(* directory restart → gateway.reconnect())
```

**Process exit.** `run.rs` waits on `select!{ signals => Exit::Ok, f = engine.fatal() => { log f; Exit::Internal } }`. Both branches then call `engine.shutdown()`. Exit code 1 lets systemd's `Restart=always` start a fresh process.

**`/healthz`:**
- 200 with `{status: "ok" | "degraded", actors: [...]}`. "degraded" means some actor is restarting.
- 503 only on fatal, a stuck model, or recorder/moderation/gateway NotAnswering. `HealthOnFailure=kill` must not fire on transient restarts.

**System page:** a new `SystemState.actors: Vec<ActorStatus>` (`#[serde(default)]`), plus tracks and rooms counts.

### 2.6 Shutdown

`Phase { Running, StopIntake, FlushTracks, FinishLive, LeaveVoice, CloseRooms, DrainLog, CloseGateway, JoinModels, Stopped }` is published on a watch.

The budget is `ShutdownBudget { total: 17s, flush: 2s, finish_live: 6s, leave: 3s, close_rooms: 2s, drain_log: 3s, close_gateway: 1s, models: 1s }`; each phase gets `min(own, remaining)`. The quadlet has `StopTimeout=20`, so the web stop in `run.rs` drops from 10 s to 2 s; or raise StopTimeout to 40.

`Engine::shutdown(&self)` keeps its signature, runs once (`OnceCell`), and does:

1. **Stop intake.** Set the phase:
   - Control disables the follow machine (no new joins) and drops chat commands.
   - Rooms stop subscribing.
   - Web mutations return `EngineError::ShuttingDown`.
   - Digest and undo start no new work; pending undos stay in the index.
2. **Flush tracks.** `control → room: Flush` → each track gets `Stop(FlushReason::End)`, which maps to `CutCause::Shutdown`. Open speech is cut, the scorer input closes, and the room awaits its tracks.
3. **Finish live work, with a deadline.**
   - Await the scorers (classification finishes).
   - `moderation.ask(Barrier)` guarantees every Heard has been decided and its record/play/followup enqueued.
   - `room: Drain` waits for the playback queue.
   - `enforcer: Drain` waits for actions and reports.
   - Past the deadline, queued play items are recorded as `PlayOutcome::Failed{"the bot stopped before it was said"}`. Unsent reports become `MessageSent{ok:false}` and unrun actions `Action{Failed{"the bot stopped"}}`. Nothing disappears silently.
4. **Leave voice while the gateway is still open.** Control runs `machine.shutdown()`; the Leave ops go through the ordered sender with the existing retry, and it awaits the drain.
5. **Close rooms.** `RoomCmd::Close` → `VoiceRoom::close()`; await the room JoinSet.
6. **Drain the log writer.** `recorder.record(Stopped{clean})`, where `clean` is false if a phase timed out or a fatal happened. Then `barrier()`; the recorder exits.
7. **Close the gateway** (`ctl.close()`); the gateway actor exits.
8. **Join the model threads off the runtime:** `timeout(budget, spawn_blocking(move || inference.shutdown()))`. `Inference::shutdown` is idempotent, so `Rig::stop` keeps working. `run.rs` stops calling it on the runtime thread.

At the end: `root.cancel()`, `tracker.close()`, a bounded `tracker.wait()`, and the phase report is logged.

### 2.7 The moderation pipeline (`decide.rs`, `evidence.rs`, `enforcer.rs`, `recorder.rs`)

```rust
pub(crate) fn decide(st: &mut ModState, eff: &Effective, still_tracked: bool, h: &Heard, scored: &Scored,
                     guilds: &Guilds, session: &SessionView, now: Timestamp, now_mono: f64, model: &str) -> Decided;
pub(crate) struct Decided { record: SentenceRecord /*audio None*/, card: SentenceCard, counts: Counts,
  violation: Option<ViolationItem>, play: Option<PlayItem>, followup: Option<(Option<EscalationStep>, ClfLang)>,
  keep_audio: bool, attach_modlog: bool, attach_owner: bool }
pub(crate) struct Evidence { pcm: Arc<[f32]>, wav: OnceLock<Bytes>, stored: tokio::sync::OnceCell<Result<BlobInfo, StoreError>> }
impl Evidence { fn wav(&self) -> Bytes; async fn store(&self, blobs: &dyn BlobStore) -> Result<BlobInfo, StoreError>; }
```

`decide` is pure. In the moderation loop, all steps below are synchronous and **nothing is awaited between them**:
1. Run `decide`.
2. Create `Evidence` only if `keep_audio || attach_modlog || attach_owner`. The WAV is built lazily, once, and shared between the blob store and the reports.
3. **`recorder.sentence(record, evidence_if_kept)` first**, then **`room.play(item)`**. Enqueuing first guarantees Sentence comes before Played, even for an instant `TooLate` or `NothingToSay`.
4. Publish the live deltas: card, counts, violation.
5. `enforcer.send(Followup)`.
6. `speech.prerender(next predicted utterances)`.

The recorder writes the blob concurrently, then commits `[BlobAdded?, Sentence{audio}]` as one batch, in order. On a blob failure the Sentence is recorded with `audio: None`, as today.

The enforcer runs per person, in FIFO order:
1. `step_action`: `patch_member`, then `recorder.record(Action)`, then `undo.send` (only after the enqueue).
2. Announce the action.
3. Run the modlog and owner DM reports. Attachments use `evidence.wav()`. `MessageSent` is fire-and-forget.

Different people run concurrently.

### 2.8 Deadline-aware inference queue (this belongs in pb-infer)

The policy has to live where the model thread pops jobs. A queue in the engine would have to hold jobs back and track whether the worker is free, which is the wrong layer. The engine supplies only the inputs: the deadline (from the per-person `max_reaction_delay`) and a flow key.

**Additive changes to pb-infer v1:**

```rust
pub struct Job { pub priority: Priority, pub deadline: Option<std::time::Instant>, pub flow: Option<u64> }
pub struct SpeakJob { pub priority: SpeakPriority, pub deadline: Option<std::time::Instant> }
impl Inference { pub async fn classify_job(&self, pcm: Arc<[f32]>, job: Job) -> Result<Scored, InferError>;
                 pub async fn speak_job(&self, voice: &str, text: &str, opts: SpeakOpts, job: SpeakJob) -> Result<Speech48, InferError>; }
// classify()/speak() delegate with deadline None, flow None. Queue::push_job(prio, deadline, flow, item).
// QueueStats gains `late: u64`.
```

**How the queue picks the next job, within one priority:**
- Only the **head job of each flow** is a candidate. A flow is per (guild, user), which keeps per-person decision order.
- Classes, in order: on-time jobs earliest-deadline-first; then jobs without a deadline, FIFO; then late jobs, oldest first.
- Late jobs are **never dropped**. When both late and on-time jobs are waiting, one pick in four goes to the oldest late job, so late jobs always progress.

**Time base.** The engine converts with `tokio::time::Instant::into_std()`. Under paused tokio time this is only meaningful for ordering, so lateness tests run in real time.

### 2.9 Caches

**Settings (`SettingsView`)**, rebuilt eagerly on every swap; invalidation happens by construction:

```rust
pub struct SettingsView { pub version: u64, tree: Arc<SettingsTree>, global: Arc<Effective>,
  servers: HashMap<GuildId, ServerView { eff: Arc<Effective>, allowed: bool, listed: Arc<BTreeSet<UserId>>, tracked: Arc<BTreeSet<UserId>> }>,
  people: HashMap<(GuildId, UserId), Arc<Effective>> /* only non-empty person layers */ }
impl SettingsView { pub fn tree(&self)->&Arc<SettingsTree>; pub fn effective(&self, Option<GuildId>, Option<UserId>) -> Arc<Effective>;
  pub fn guild_allowed(&self,g)->bool; pub fn listed_for(&self,g)->Arc<BTreeSet<UserId>>; pub fn tracked_for(&self,g)->Arc<..>; pub fn is_tracked(&self,g,u)->bool; }
```

Missing layers fall back to server, then global. That is exactly what `SettingsTree::effective` computes, including the pause rule and the `source` fields; a unit test compares every combination.

**Speech (`speech.rs`).**

The key is `SpeechKey { voice: Arc<str>, rate_milli: u32, text: Arc<str> }`, the full filled text. Lookup order:
1. Memory LRU by bytes (`AudioCache`, 64 MiB). Fix: an entry larger than the budget is not inserted.
2. In-flight single-flight: `Mutex<HashMap<SpeechKey, Arc<Flight{prio, cell: tokio::sync::OnceCell<Result<..>>}>>>`. If the waiting caller has a higher priority than the in-flight job, it starts its own render at its priority and replaces the entry. Failed entries are removed so the next call retries.
3. Disk: a new **`RenderCache`** trait in pb-store-api v1 (additive; contract tests):

   ```rust
   async fn get(&self, &RenderKey) -> Result<Option<Bytes>, StoreError>;
   async fn put(&self, &RenderKey, Bytes) -> Result<(), StoreError>;
   async fn prune(&self, PruneFilter) -> Result<u64, StoreError>;
   fn size(&self) -> u64;
   ```

   `RenderKey = sha256("pb-speech/1" ‖ voice id ‖ language ‖ sample_rate ‖ quality ‖ rate_milli ‖ sentence_gap_ms ‖ text)`. pb-store gets `FsRenderCache` at `/data/cache/tts/aa/<hex>` (temp file + rename, a header with length and checksum, no fsync, no cap, as `docs/design.md` §2 says) and `MemRenderCache`.

   Writes are fire-and-forget tracked tasks. A disk read happens only on a memory miss and is always cheaper than the TTS render it replaces.
4. TTS through `speak_job`.

Clips use the same pattern: a second `AudioCache<BlobHash>`, single-flight, and decode/resample in `spawn_blocking`.

```rust
pub(crate) struct UtteranceCtx { guild: GuildId, channel: Option<ChannelId>, person: Option<UserId>, line: Line,
  exact: Option<(Lang, String)>, heard: Option<ClfLang>, label: Option<Label>, fields: Fields }
pub(crate) enum NoRepeatMode { Remember /*playback*/, Ignore /*preview, prerender*/ }
impl SpeechService { async fn render(&self, &UtteranceCtx, SpeakPriority, Option<Instant>, NoRepeatMode) -> Result<Rendered, RenderError>;
  fn prerender(&self, Vec<UtteranceCtx>); fn forget_clip(&self, &BlobHash); fn clear_speech(&self); }
```

**Prerender keys equal live keys.** Both build their context with the same `fn warning_ctx(...)`:
- count = `Prediction.next_count`; step and action from `escalation.step_for(next_count)`;
- strikes from a new additive `Decider::strikes(g, u, now, window)` in pb-policy;
- channel = the person's current voice channel;
- languages: the fixed language, or with `auto` the last heard language plus the first fallback.

Prerender is triggered:
- when listening starts;
- after **every** decision for that person;
- when a tracked person shows up in voice (prerenders the greeting).

Prerender ignores no-repeat and only pre-decodes clip candidates.

**Names and phrases rendered apart: no.** Keep full-sentence renders and fix only the key and the prerender.
- **Piper treats each sentence as independent.** `crates/pb-tts-piper/src/lib.rs` `synthesize` splits the text into espeak sentences, runs one VITS inference per sentence, peak-normalises each sentence on its own, and joins them with a 150 ms gap. A fragment such as "Hey", "Richard" or "3" would be voiced as a complete sentence: falling final intonation, loudness normalised separately, no coarticulation, and silence at both edges. The join is clearly audible.
- **The existing splicing is a different case.** The 120 ms splice in pb-voicelines `plan.rs` is acceptable only because the name there is a recording.
- **Sentence-level reuse is free of artifacts, because it is what Piper already does.** An optional step adds a sentence memo inside pb-tts-piper, keyed by (voice, speaker, scales, phoneme ids) and owned by the TTS thread, so "Keep it clean." is rendered once for everyone.
- **To confirm by ear:** add `crates/pb-tts-piper/examples/splice.rs`, which writes the whole-sentence render and the fragment-joined render as two WAVs to compare.
- Renders are random (default noise scales), so a cache freezes one take; that is fine.

### 2.10 Clock, typed errors, tracing
- **Clock.**
  - `SystemClock` keeps its name and `Default`: `mono` now comes from `tokio::time::Instant`; `now` stays `Timestamp::now()`.
  - New public `TokioClock::new(base: Timestamp)`: wall time = base + tokio elapsed, for paused-time tests.
  - All `std::time::Instant` uses in the engine are removed (redirect memo, cells sweep).
- **Typed errors.**
  - `EngineError` changes:
    - `Render(RenderError)` (was a String);
    - `Fluxer(FluxerError)` (was a String);
    - new `Login(LoginError)` and `ShuttingDown`.
  - New `RenderError { NoVoice{lang}, Tts(InferError), ClipMissing(BlobHash), ClipUnreadable{clip, error}, Store(StoreError), ShuttingDown }`.
  - `discover`, `login_endpoints` and `oauth_user` return `Result<_, EngineError>`.
- **Spans.**
  - `session{attempt, bot}`, `room{guild, channel}`, `track{user}`, `actor{name, attempt}`;
  - `sentence{id, no, guild, user}`, created at segment Open and carried through ScoreJob → Heard → PlayItem → Followup → `RecordMsg::Sentence`;
  - events `scored`, `decided`, `played`, `action`, `reported`.

### 2.11 Tests and fakes
1. **Pure unit tests:**
   - `decide`;
   - `SettingsView` matches `tree.effective` everywhere;
   - LRU, including the oversize fix;
   - `warning_ctx` live and prerender produce the same `SpeechKey`;
   - backoff and escalation;
   - the pb-infer queue (EDF, flows, late class, fairness).
2. **Actor tests with paused time** (`#[tokio::test(start_paused = true)]`, in-crate `#[cfg(test)] mod fakes`: `MemLog` with a release gate, `MemBlobs` with a delay, `StubIndex`, `RecordingCtl`, a fake TTS port behind `pub(crate) trait Tts` implemented for `Inference`). Cases:
   - the recorder keeps FIFO across producers, batches, acks, and returns errors without hanging when the log is halted; barriers work;
   - **a warning is played while the Sentence append is still blocked**;
   - actions keep per-person order;
   - the supervisor restarts, keeps the mailbox, and escalates to fatal;
   - undo and digest work on virtual time;
   - a burst of 50 renders makes one TTS call.
3. **Black-box scenario tests**, not ignored, in real time: `crates/pb-engine/tests/scenarios.rs`. They use pb-fluxer-fake + pb-fluxer, an extended MemVoice, pb-store on a tempdir, and `Inference` with fake models.
   - **Ported from e2e:** warn and report; benign; strikes → mute → unmute; no Speak permission → chat + greeting; gateway resume; German commands.
   - **New:** late verdict (classifier held back → Late, not voiced); prerender hit (the TTS log shows no Live render); a slow disk (`SlowLog`/`SlowBlobs` wrappers) with prompt playback; the shutdown order (Sentence with `CutCause::Shutdown`, `bot_connections()` empty while the gateway is open, `Stopped{clean:true}` last).
   - The LiveKit/real-model tests and `parity.rs` stay `#[ignore]` in `crates/pb/tests`.
4. **pb-testkit additions:**
   - `memvoice`: `Mic::{leave, mute}`; `MemVoice::{audience, deny_speak, disconnect, played_items}`; `Mic::say` timed with tokio `Instant`.
   - New `fakemodels` feature (depends on pb-models-api): `ToneVad` (RMS threshold); `ToneClassifier` (pitch from zero crossings: 440 Hz benign, 880 Hz profane, plus a std Condvar `Gate` and a call counter); `BeepTts` (en/de voices, deterministic length, call log); `tone(freq, secs, rate, amp)`.

---

## 3. Migration steps (each one compiles and keeps tests green)

1. **Baseline.** Commit the in-flight tree and fix `AudioCache::insert` for oversized entries.
   Files: the 16 files in `git status`, plus `crates/pb-engine/src/v1/audio_cache.rs`.
2. **Test harness first.**
   - pb-testkit `fakemodels` and the MemVoice extensions.
   - `xtask/layers.toml`: `[dev_allow] L3 = ["L2"]` (the same exception L4 already has; scenarios need pb-store and pb-fluxer).
   - `docs/proposals/0004-engine-actors.md`.
   - pb-engine dev-deps (pb-testkit, pb-fluxer-fake, pb-fluxer, pb-store, tempfile, tokio `test-util`).
   - `tests/common/mod.rs` and `tests/scenarios.rs` pinning today's behaviour.

   Files: `crates/pb-testkit/{Cargo.toml,src/lib.rs,src/memvoice.rs,src/models.rs}`, `xtask/layers.toml`, `docs/proposals/0004-engine-actors.md`, `crates/pb-engine/{Cargo.toml,tests/common/mod.rs,tests/scenarios.rs}`.
3. **Clock on tokio time,** plus `TokioClock`.
   Files: `deps.rs`, `engine.rs`, `cells.rs`, `mod.rs`.
4. **Typed errors.**
   Files: `error.rs`, `speak.rs`, `engine.rs`, `room.rs`, `library.rs`.
   Callers: `crates/pb-web/src/fmt.rs` (`engine_error`), `crates/pb-web-server/src/{login.rs,setup.rs}`, `crates/pb-i18n/locales/{en,de}/*.ftl` (`err-shutting-down`, `err-login`).
5. **`SettingsView`,** watch-backed, writes still serialized the old way; mechanical switch from `current().effective(..)` to `view()`.
   Files: `settings.rs` and every engine module.
6. **Runtime primitives,** with unit tests but not yet wired in.
   Files: workspace `Cargo.toml` (tokio-util 0.7.19 with `rt`), pb-engine `Cargo.toml` (tokio-util, futures), `mailbox.rs`, `supervise.rs`, `health.rs`.
7. **Supervise the 7 long-lived tasks** with their current logic (moderation and undo mailboxes owned by the supervisor; the moderation restart reseeds after `caught_up(log.head())`). Add `health()`/`fatal()`.
   Files: `engine.rs`, `moderation.rs`, `actions.rs`, `reports.rs`, `cells.rs`, `mod.rs`.
8. **Expose liveness.**
   - `SystemState.actors`: `crates/pb-live-proto/src/v1/state.rs`.
   - System page: `crates/pb-web/src/islands/system.rs` and the i18n files.
   - `/healthz`: `crates/pb-web-server/src/server.rs`.
   - Fatal select: `crates/pb/src/run.rs`.
9. **Recorder.** Replace every `core.record(..).await`:
   - Fire-and-forget: Played, names (the spawn is removed), Action, MessageSent, PersonSeen, JarReset, Started.
   - Acked: library, `Engine::record`, settings audit.
   - Digest awaits ack and catch-up.

   Files: `recorder.rs`, `core.rs`, `engine.rs`, `moderation.rs`, `room.rs`, `actions.rs`, `reports.rs`, `commands.rs`, `people.rs`, `control.rs`, `library.rs`, `settings.rs`.
10. **Moderation pipeline:** pure `decide`, `Evidence`, `RecordMsg::Sentence` pipelining, `enforcer.rs`, jar and counters owned by moderation, queries replacing `Core.jar`, reseed with barrier.
    Files: `moderation.rs`, `decide.rs`, `evidence.rs`, `enforcer.rs`, `recorder.rs`, `actions.rs`, `reports.rs`, `commands.rs`, `cells.rs`, `engine.rs`, `core.rs`.
11. **Deadline-aware pb-infer queue,** then the ordered per-track scorer (the per-sentence spawn goes away).
    Files: `crates/pb-infer/src/v1/{queue.rs,mod.rs}`, `crates/pb-infer/tests/infer.rs`, `track.rs`. Optional: `QueueStatus.late` in pb-live-proto and pb-web.
12. **Directory actor and `Arc<GuildInfo>`.** Remove `Core.guilds` and `names_recorded`; commands use `synced()`.
    Files: `directory.rs`, `guilds.rs`, `control.rs`, `people.rs`, `engine.rs`, `commands.rs`, `cells.rs`, `speak.rs`, `reports.rs`, `moderation.rs`, `core.rs`.
13. **Gateway actor and `SessionView`.** Remove `ctl`, `connection` and `redirects` (the latter becomes a leaf memo).
    Files: `gateway.rs`, `engine.rs`, plus every `ctl()` user.
14. **Control owns `VoiceWorld`, the room registry and `RoomsView`;** ordered voice-state sender; commands as tracked tasks. Remove `voice`, `rooms`, `conns`, `speaking` and `listening`.
    Files: `control.rs`, `room.rs`, `cells.rs`, `commands.rs`, `actions.rs`, `library.rs`, `engine.rs`, `core.rs`.
15. **Room, playback and track ownership:** participants watch, echo watch, playback as the only audience writer, `TrackCmd`, `JoinSet`, moderation counters (remove `sentence_no`), room/track/sentence spans.
    Files: `room.rs`, `playback.rs`, `track.rs`, `moderation.rs`, `core.rs`.
16. **Library actor** (remove `Core.clips`).
    Files: `library.rs`, `speak.rs`, `engine.rs`.
17. **Settings actor** (`F: Send + 'static`; drop the tokio Mutex).
    Files: `settings.rs`. Callers: `crates/pb-web-server/src/setup.rs`, the two closures at about :228 and :279 need `move`.
18. **`RenderCache`:** the trait and contract tests in pb-store-api; Fs and Mem implementations in pb-store; `Deps.render_cache`; `pb cache prune`.
    Files: `crates/pb-store-api/src/v1/{render_cache.rs,mod.rs,contract.rs}`, `crates/pb-store/src/v1/{render_cache.rs,mod.rs}`, `deps.rs`. Callers: `crates/pb/src/{run.rs,main.rs,tools.rs}`, `crates/pb/tests/common/mod.rs`, `crates/pb-web-server/tests/common/mod.rs`, `crates/pb-engine/tests/common/mod.rs`.
19. **`SpeechService`:** single-flight, disk tier, `warning_ctx` and prerender triggers, `Decider::strikes`, no-repeat modes. Remove `speech`, `clip_pcm` and `no_repeat`.
    Files: `speech.rs`, `speak.rs`, `audio_cache.rs`, `moderation.rs`, `room.rs`, `playback.rs`, `engine.rs`, `crates/pb-policy/src/v1/decide.rs`. Optional: the pb-tts-piper sentence memo and the `splice` example.
20. **Views actor and `Cells` from World.** Remove `dirty`; delete `core.rs` in favour of `ctx.rs`.
    Files: `views.rs`, `cells.rs`, `live.rs`, `system.rs`, `engine.rs`, `mod.rs`.
21. **Structured shutdown** (phases and budget), plus the scenario test.
    Files: `engine.rs`, `gateway.rs`, `control.rs`, `room.rs`, `playback.rs`, `track.rs`, `moderation.rs`, `enforcer.rs`, `recorder.rs`, `crates/pb/src/run.rs` (stop calling `inference.shutdown()` on the runtime; web stop 2 s), optionally `deploy/quadlet/profanity-watch.container` (StopTimeout).
22. **Enforcement and docs.**
    - A crate-local `crates/pb-engine/clippy.toml` (copying the root keys) with `disallowed-methods` for `tokio::spawn` and `std::time::Instant::now` and `disallowed-types` for `std::sync::RwLock`.
    - Update `docs/design.md` §3.

### Public API changes and every caller
- **`Engine::{discover, login_endpoints, oauth_user}` errors:** `crates/pb-web-server/src/setup.rs` (~:219), `crates/pb-web-server/src/login.rs` (~:145, ~:249); `e.into()` becomes `e.to_string().into()`.
- **`EngineError` variants:** `crates/pb-web/src/fmt.rs` `engine_error`.
- **`SettingsService::change` bound:** `setup.rs` (2 sites). `forms.rs:209`, `voice.rs:71`, commands and people are already `move`/`Send`.
- **`Deps.render_cache`:** the four construction sites listed in step 18.
- **`Guilds`/`GuildInfo` field types (`Arc` inside):** source-compatible for every caller. pb-web only uses methods, `.get(g).cloned()` and `.channels.values()` (`pages/community.rs` ~:115, `pages/settings.rs` ~:170).
- **New:** `Engine::health`, `Engine::fatal`, `SettingsService::view`, and the types `EngineHealth`, `ActorHealth`, `ActorState`, `FatalError`, `RenderError`, `SettingsView`, `TokioClock`. Used by `server.rs` and `run.rs`.
- **Unchanged:** `Engine::shutdown` (same signature; now also joins the models off the runtime). Callers: `run.rs`, `pb/tests/common` `Rig::stop`, the pb-web-server test harness.
- **Additive elsewhere:** pb-infer (`Job`, `SpeakJob`, `classify_job`, `speak_job`, `QueueStats.late`), pb-store-api (`RenderCache`), pb-policy (`Decider::strikes`), pb-live-proto (`SystemState.actors` with serde default).

---

## 4. Risks and the guarantees that must hold

**Ordering guarantees, and the code that relies on them today:**
1. **Sentence before the Played/Action/MessageSent that refer to it.** Today `moderation.rs` `decide` awaits the record before `room.play`. New: the Sentence is enqueued before the PlayItem and the Followup.
2. **The original mute Action before its undo.** The undo updates the `actions` row by id (`crates/pb-store/src/v1/index/apply.rs`, the Action arm). `actions.rs` `step_action` records before `undo.send`; keep enqueue-then-send.
3. **BlobAdded in the same batch as its Sentence; BlobDeleted after it.** `apply.rs` updates sentences by audio hash.
4. **Jar: JarReset and Sentence{jar} in one order for memory and index.** `apply.rs` `add_jar` and the JarReset arm. Fixed by letting moderation emit both.
5. **Per-person decision order** (Decider strikes, Violations counts). Today it depends on the comment in `track.rs` ("the classifier queue keeps the order") and `crates/pb-infer/src/v1/queue.rs` `Ord`. New: per-flow FIFO in the queue plus the ordered scorer.
6. **Per-person name order.** `control.rs` `record_names`; fixed by directory → recorder FIFO.
7. **The index catches up before seeding** (`engine.rs` `Engine::start`, working tree). Restarts use barrier then `caught_up`.
8. **Digest idempotence:** `reports.rs` `digest_once` reads `index.last_digest()`, which `apply.rs` builds from the MessageSent Digest arm. Await the ack and catch-up after sending.
9. **A clip is usable only once it is durable:** `library.rs` records, then `put_clip`; keep commit-after-ack.
10. **Settings:** files written, then swap, then audit (`settings.rs` `change`).
11. **The echo guard is set before the audio is heard** (`room.rs` `play_one` begin/end; `track.rs` `handle` overlap check). The watch send must happen before `out.play`.
12. **Leave ops are retried while the gateway reconnects** (`control.rs` `exec`), and **follow-machine RoomUp/RoomDown order** per channel.
13. **`Started` first and `Stopped` last** in each run's events.

**Risks:**
- **Supervision depends on unwinding.** If `panic = "abort"` is ever set, a crash means a process restart; document it in `Cargo.toml`.
- **Paused time and OS threads** (pb-infer, log writer, index) make tests flaky. Keep paused tests on in-crate fakes; scenarios run in real time.
- **Mailboxes have no cap,** by the no-caps rule. NotAnswering detection is the safety net.
- **Directory lag** behind gateway events. Commands use `synced()`; the cells refresh tolerates a few milliseconds.
- **Playback no longer waits for the Played fsync,** so a "Say now" reply can arrive before the record is durable. Records still keep their order.
- **Disk-cache staleness** if a voice is replaced under the same id. The key includes language, rate and quality; otherwise `pb cache prune`.
- **Shutdown must fit in StopTimeout=20,** and Fluxer outages during the leave are cut off by the phase deadlines.
- **Prerender load** runs at the lowest priority and is deduplicated by the LRU and single-flight.
- **The person page can show jar 0 for a moment** before `FillPerson` arrives, except in SSR, which awaits the fill.
- **pb-store's index follower** (a bare `tokio::spawn`, `index/mod.rs:92`) stays unsupervised. It is outside the engine; give it its own restart loop later.
- **The working tree is still changing.** Re-diff before starting step 1.
