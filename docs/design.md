# pb — the Rust rebuild of the Fluxer profanity-watch bot: design

Status: approved 2026-10-03. This document is the reference for the rebuild. Every decision here can be changed by a
written proposal in `docs/proposals/` (number, the user's words, options, decision, why, how to undo).

## 0. Ground rules

- All hand-written code is Rust (plus CSS, Fluent `.ftl`, TOML). Generated files (wasm-bindgen glue) are allowed; CI lists
  every JavaScript file the browser receives and fails on anything hand-written.
- Dependencies are pure Rust unless listed in `docs/exceptions.toml` with reason, scope and review date. Tools are chosen
  only on capability and technical fit; age and popularity never decide.
- Every part is its own crate behind a defined interface (`pub mod v1`, re-exported at the crate root). A change adds `v2`
  beside `v1` with an adapter; versions are never removed. Each interface crate ships a contract-test kit (feature
  `contract-tests`) that every implementation, including replacements and fakes, must pass.
- No arbitrary caps. Only limits imposed by the model (classifier input ≤ 30 s) or the platform (Fluxer rate limits,
  message and attachment sizes) exist; rate limits queue and never drop; lists paginate; data is kept until someone deletes
  it on purpose.
- Fluxer only. No Discord support and no abstractions for it.
- Settings precedence: person > server > global > `config.toml`/environment > built-in default. Everything a person sets
  is set in the web UI (or chat commands); `config.toml` is optional.

## 1. Workspace

Layers (enforced by `cargo xtask deps`):

| Layer | Crates | May depend on | Must not depend on |
|---|---|---|---|
| L0 pure core (sync, no I/O, wasm-safe) | pb-domain, pb-settings, pb-segment, pb-policy, pb-voicelines, pb-commands, pb-i18n, pb-live-proto, pb-wordlist | each other (DAG), serde, jiff, toml_edit, fluent, thiserror | tokio, futures, `async fn`, axum, burn, livekit, rten, turso |
| L1 interfaces | pb-models-api, pb-fluxer-api, pb-voice-api, pb-store-api | L0, `tokio::sync`, async-trait | concrete adapters |
| L2 adapters / compute | pb-audio, pb-vad-silero, pb-classifier-roblox, pb-tts-piper, pb-infer, pb-fluxer, pb-voice-livekit, pb-store, pb-import, pb-weights | L0, L1, own third-party crates | pb-engine, web crates |
| L3 application | pb-live, pb-engine | L0, L1, pb-infer, pb-audio | concrete adapters (pb-fluxer, pb-voice-livekit, pb-store) |
| L4 delivery | pb-web (`ssr`/`hydrate`), pb-web-server | `hydrate`: L0 only; `ssr`/server: L3 handles + L1 | concrete adapters |
| L5 binaries and tooling | pb, xtask, pb-testkit (dev), pb-fluxer-fake (dev), tools/pb-fetch | everything | — |

Extra rules: `livekit` only in pb-voice-livekit; `burn` only in pb-classifier-roblox; `rten` and `espeak-ng` only in
pb-tts-piper (pb-vad-silero uses rten's ONNX reader and SIMD crates, not the runtime); `turso` only in pb-store and
pb-import; only `pb` wires concrete implementations.

| Crate | Responsibility |
|---|---|
| pb-domain | ids (`GuildId`, `UserId`, `ChannelId`, `ConnectionId`, `SentenceId`), `Label` (8), `ClfLang` (30), `Lang`, `Scope`, `Score`, `Verdict`, `Decision`, `EscalationStep`, `BlobHash`, `Audience`, `ActionKind`; time = `jiff::Timestamp` |
| pb-settings | typed schema (one declaration), value newtypes, validation, layered resolution with source, TOML parse/edit (toml_edit, never `fmt()`), migrations, generated `SettingKey`, docs generator |
| pb-segment | `FrameAssembler` (512-sample frames), `PcmRing`, hysteresis `Segmenter` (exact port of `vendor/roblox_vc/app/segmenter.py`), `EchoGuard`, 30 s windowing |
| pb-policy | `Decider` (strikes, observe-only, late decisions, escalation step), `FollowMachine` (port of `follow.py`), desired-channel policy, action planning |
| pb-wordlist | `WordList`: listed words and phrases in a text despite case, look-alikes, invisible, stretched and spaced-out letters |
| pb-voicelines | line catalogue, slots, resolution, templates, `UtterancePlan`, no-repeat picker, next-utterance prediction |
| pb-commands | `!pb` parser and permission levels → `CommandPlan` |
| pb-i18n | Fluent bundles de/en compiled in, for bot text and UI |
| pb-live-proto | live wire types, client reducer (`TopicTracker`), conveyor `display_stage`, clock offset |
| pb-models-api | `VadModel`, `Classifier`, `TtsEngine` + contract kits |
| pb-fluxer-api | `Fluxer`, `FluxerCtl`, `GatewayEvent`, `VoiceGrant` (`#[non_exhaustive]`), `VoiceStateOp`, `Destination`, `MemberPatch` |
| pb-voice-api | `VoiceTransport`, `VoiceRoom`, `AudioOut`, `RoomEvent`, `TrackKey` |
| pb-store-api | `EventLog`, `Index`, `Blobs`, `SettingsRepo`, `SessionRepo`; `Event` enum (log schema v1); typed queries |
| pb-audio | decode (symphonia + opus-decoder wrapper), WAV (hound), resample (rubato), loudness (ebur128), limiting, fades |
| pb-vad-silero | Silero v6.2, a hand-written forward pass (weights from the official `silero_vad.onnx`, SIMD chosen at run time), batched across streams; energy-gate fallback |
| pb-classifier-roblox | Roblox voice-safety-classifier v3 in Burn (burn-flex; feature `gpu` = burn-wgpu plus its own CubeCL kernels for linear layers, layer norm and attention) |
| pb-tts-piper | Piper VITS on rten, espeak-ng phonemes, piper id mapping, resample to 48 kHz |
| pb-infer | OS threads owning the models (one per speech model; a voice `<model>:<id>` goes to its model), priority queues, live thread-count changes, metrics, `Inference` handle |
| pb-fluxer | discovery, gateway (hello, identify, heartbeat, resume, op 7/9, close codes), op 4/op 3 pacing, REST with header-driven rate-limit queues, OAuth2 PKCE |
| pb-voice-livekit | `VoiceTransport` on `livekit` (exceptions register) |
| pb-store | hash-chained JSONL log, Turso index, blob store, TOML settings repo, sessions; in-memory implementations |
| pb-import | one-time import of the Python bot's `bot.db`, `secrets.json`, `clips/`, `evidence/` |
| pb-weights | pinned manifest (URL, revision, sha256, licence), resumable download, verify |
| pb-live | server hub: topic cells, snapshots, deltas, per-connection session machine, write deadlines |
| pb-engine | supervisor, Fluxer session lifecycle, voice world, follow driver, room/track/playback tasks, moderation actor, reports, commands, pre-render, `EngineApi` v1 |
| pb-web | Leptos app (routes, SSR components, islands, server functions, stylance CSS) |
| pb-web-server | axum assembly: host allowlist, CsrfLayer, sessions, OAuth, uploads, media routes, live websocket |
| pb | clap binary: `run`, `fetch-weights`, `import`, `store verify|rebuild-index|compact`, `settings check|docs`, `setup-code`, `health`, `doctor`, `bench`, `tls generate`, `cache prune` |
| xtask | CI gates: zero-C, shipped JS, deps rules, tokei, golden runner, container smoke |
| pb-testkit | local livekit-server, HS256 token minting, scripted participants, golden loaders, browser harness |
| pb-fluxer-fake | fake Fluxer (REST, gateway, OAuth, pending-join and confirm semantics, resume buffer) |

### Key interfaces (v1, abridged)

```rust
// pb-fluxer-api::v1
#[async_trait] pub trait Fluxer: Send + Sync + 'static {
    async fn discover(&self, instance: &Url) -> Result<Endpoints, FluxerError>;
    async fn login(&self, ep: &Endpoints, token: &Secret<String>)
        -> Result<(Arc<dyn FluxerCtl>, mpsc::UnboundedReceiver<GatewayEvent>), LoginError>;
}
#[async_trait] pub trait FluxerCtl: Send + Sync {
    fn me(&self) -> &BotIdentity;
    async fn voice_state(&self, op: VoiceStateOp) -> Result<(), FluxerError>;   // Join | Confirm{conn} | Leave{conn}
    async fn presence(&self, text: Option<String>);                               // latest value wins
    async fn send(&self, to: Destination, m: OutgoingMessage) -> Result<MessageId, FluxerError>; // queued, never dropped
    async fn react(&self, m: MessageRef, emoji: &str) -> Result<(), FluxerError>;
    async fn patch_member(&self, g: GuildId, u: UserId, p: MemberPatch) -> Result<(), FluxerError>;
    async fn member(&self, g: GuildId, u: UserId) -> Result<Option<Member>, FluxerError>;
    async fn application(&self) -> Result<Application, FluxerError>;
    fn limits(&self) -> MessageLimits;
    async fn close(&self);
}
#[non_exhaustive] pub enum VoiceGrant {
    LiveKit { guild: GuildId, channel: ChannelId, connection: ConnectionId, endpoint: Url, token: Secret<String>,
              e2ee_key: Option<Secret<String>> } }

// pb-voice-api::v1
#[async_trait] pub trait VoiceTransport: Send + Sync + 'static {
    async fn connect(&self, g: &VoiceGrant, o: ConnectOpts)
        -> Result<(Box<dyn VoiceRoom>, mpsc::UnboundedReceiver<RoomEvent>), TransportError>; }
#[async_trait] pub trait VoiceRoom: Send + Sync {
    fn participants(&self) -> Vec<Participant>;
    async fn subscribe(&self, t: &TrackKey) -> Result<AudioIn, TransportError>;   // i16 mono 16 kHz chunks
    async fn unsubscribe(&self, t: &TrackKey) -> Result<(), TransportError>;
    async fn publish_voice(&self) -> Result<Box<dyn AudioOut>, TransportError>;   // Err(NoSpeakPermission)
    async fn set_audience(&self, a: &AudienceSet) -> Result<(), TransportError>; // All | Only(identities)
    async fn close(&self); }
#[async_trait] pub trait AudioOut: Send {
    async fn play(&mut self, pcm48: &[i16]) -> Result<(), TransportError>;      // returns after playout
    fn stop(&mut self); }

// pb-models-api::v1 — each instance is owned by one thread (Send, deliberately not Sync)
pub trait VadModel: Send + 'static {
    fn info(&self) -> VadInfo;
    fn new_state(&self) -> VadState;
    fn step(&mut self, frames: &[[f32; 512]], states: &mut [&mut VadState]) -> Vec<f32>; }
pub trait Classifier: Send + 'static {
    fn info(&self) -> &ClassifierInfo;
    fn classify(&mut self, pcm16k: &[f32]) -> Result<RawScores, ModelError>;     // batch 1; sigmoid[8], softmax[30]
    fn set_threads(&mut self, n: NonZeroUsize) -> Result<(), ModelError>; }
pub trait TtsEngine: Send + 'static {
    fn voices(&self) -> &[VoiceInfo];
    fn synthesize(&mut self, v: &VoiceId, text: &str, rate: f32) -> Result<Pcm48k, TtsError>; }

// pb-store-api::v1
#[async_trait] pub trait EventLog: Send + Sync {
    async fn append(&self, ev: Vec<NewEvent>) -> Result<Vec<EventRef>, StoreError>; // group commit, fdatasync before Ok
    fn head(&self) -> Option<EventRef>;  fn health(&self) -> WriterHealth;
    fn scan(&self, from_seq: u64) -> BoxStream<'_, Result<StoredEvent, StoreError>>;
    async fn verify(&self) -> Result<VerifyReport, StoreError>;
    fn follow(&self) -> broadcast::Receiver<Arc<[StoredEvent]>>; }
```

## 2. Data on /data

```
/data
├── .pb.lock                  exclusive lock for the process lifetime; exit 3 if held
├── config.toml               optional (config-rs file layer): [web] [fluxer] [inference] [logging] [defaults.<setting>]
├── secrets.toml              0600, atomic replace: bot_token, client_secret, cookie_keys, setup
├── setup-code                0600, only while setup is unfinished
├── settings/global.toml      schema = 1; [settings]; [voice_lines.*]; [say_presets.*]
├── settings/servers/<gid>.toml  schema = 1; [settings]; [voice_lines.*]; [people."<uid>"] tracked …; person overrides
├── log/000001-2026-10.jsonl  event log segments (monthly rollover; the hash chain continues across segments)
├── index/index.db            Turso, derived, safe to delete
├── blobs/sha256/aa/bb/<hex>  immutable content: uploads, prepared renders, evidence
├── cache/tts/                speech render cache (derived, no size cap; `pb cache prune` by hand)
├── voices/<id>/              Piper voices installed from the UI
├── sessions.json             0600: {sha256(sid): {uid, created, expires, fresh_until, last_seen}}
├── logs/pb.log.YYYY-MM-DD    daily, no size cap
└── tmp/                      upload staging, emptied at start
```

Event log line: `{"seq":4711,"ts":"…Z","kind":"sentence","v":1,"prev":"<sha256 of previous line>","data":{…}}`; the
hash is over the exact line bytes without `\n`; genesis `prev` = 64 zeros. One writer task: drain queued appends, one
`write_all`, `sync_data`, ack. On a write/sync error (disk full included) the file is truncated back to the last good
length and the writer halts: no automatic retry; moderation continues in memory; the UI shows a banner; one retry on an
owner's request or at the next start. A torn tail is truncated at open and recorded as `log.repaired`.

Settings writes: validate → append `settings.changed` → edit the TOML document in place → temp file + fsync + rename →
swap the in-memory tree → notify the engine. Hand edits are picked up at start, SIGHUP or "Reload settings files";
invalid files → exit 78 at start (miette span), or keep old values at runtime with an error in the UI.

Importer (`pb import --from <old data dir>`; automatic on first start if `/data/bot.db` exists and the log is empty):
settings rows → TOML (removed caps reported, texts/clips → voice-line slots), history+violations → `sentence` events,
evidence and clip uploads → blobs + events, audit → `settings.changed` (source import), jar baseline, pending actions,
`secrets.json` → `secrets.toml`. Sessions and the TTS cache are not imported. A report is written.

## 3. Runtime

The engine (pb-engine, proposal 0004) is a set of actors. Each owns its state, gets work through an unbounded mailbox
and publishes what others read; nothing else is shared between them.

- **Supervisor** (tokio-util `TaskTracker` + `CancellationToken`): every long-lived actor runs under it. One that panics
  or fails starts again after a doubling pause (100 ms .. 30 s) with its state built afresh (from the index where
  needed) and its mailbox kept, so only the message in hand is lost; more than 5 crashes in 10 minutes, or a crash of a
  part that cannot be rebuilt, is fatal and `pb run` exits with 1 for the service manager to restart it. Every part's
  state, restarts and waiting work are on `/healthz` (503 when one failed or has not taken work for a minute) and the
  System page.
- **Actors:** `gateway` (one Fluxer login at a time; its session's `control` actor holds the voice world, the community
  burst, the follow machine and the rooms), `room` per call with its `playback`, `track` per microphone with its
  `scorer`, `moderation` (decides every scored sentence in order, publishes swear jars, renders a person's next
  warning ahead of time), `enforcer` (actions and reports, per person in order), `recorder` (the event log's single
  writer), `undo` (lifts timed mutes, also after a restart), `digest`, `views` (rebuilds live pages from marks),
  `threads` and `system`.
- **Published state:** what several parts read (communities, the voice world, rooms and connections, who speaks and
  listens, the clip library, swear jars, settings resolved per place) is a tokio watch of an `Arc` snapshot that its
  owner replaces; readers hold no lock. Leaf caches (rendered speech and clips, LRU by bytes) keep short critical
  sections without awaits.
- **No disk waits on the way to a warning:** moderation decides, hands the sentence (with the recording to keep) to the
  recorder and queues the warning without awaiting anything; the recorder writes in hand-over order, one append per
  batch, the recording first and in the same batch as its sentence. Only what must be durable before it is used
  (clips, recording deletions, logins, `Stopped`) waits for the acknowledgement.
- **Model threads:** `vad` (a hand-written Silero step batched over all streams), `classify` (own thread pool; within
  a priority earliest deadline first, then jobs without one, then late jobs, which are still done; one person's
  sentences stay in order; > 30 s split into 30 s windows with 25 s hop, label = max, language = duration-weighted
  mean), `tts` (Live > preview > prerender; a phrase is rendered once while several wait for it). Nothing holds a
  shared model; access only by job.
- Queues have no count caps. Lag and backlog are shown on the System page. A verdict that arrives later than
  `max_reaction_delay` (default 15 s, may be "unlimited") is recorded and counted but not voiced.
- **Shutdown**, each step with a deadline (about 17 s in all): the web UI stops (2 s); no new calls, microphones or chat
  commands; rooms cut open speech (`CutCause::Shutdown`) and wait for its scoring; moderation decides everything handed
  over; queued speech is said or recorded as not said and actions and reports finish; the bot leaves voice while the
  gateway is still open and closes its rooms; the gateway closes; `Stopped` is recorded last (unclean when a step ran
  out of time) and the log is written out; the model threads are joined off the async threads. Exit codes: 0 clean;
  1 internal fatal; 3 lock held; 78 permanent configuration problem. A missing or rejected token is a UI state, not an
  exit.

## 4. Live updates (no two-tab freeze by construction)

Topics: Sidebar, Wall, Guild(g), Person(g,u), System. Each topic is a latest-state cell plus a broadcast of deltas.
Subscribing returns a consistent snapshot; a lagging receiver gets a fresh snapshot instead of a backlog; each frame has a
10 s write deadline (a frozen client is closed at once); ping every 15 s, no pong in 45 s → close; every frame echoes the
client's `view`. Hidden tab → `Visibility{false}` → the server keeps only Sidebar; visible → `view += 1`, snapshots; frames
with an older `view` are dropped. `freeze`/`pagehide` close the socket, `resume`/`pageshow` reconnect. `AuthExpired` →
overlay "log in again", no reconnect loop. `Denied` is retried after `AccessChanged`. Client state lives in
`reactive_stores` with keyed lists (no full rebuilds); one requestAnimationFrame loop draws only visible canvases; the
conveyor station of a card is a pure function of its stage timestamps and server time, so a tab that comes back shows the
right picture immediately and never replays.

Wire protocol (pb-live-proto, JSON, `proto: 1`): client `Hello`, `Sub`, `Unsub`, `Visibility`, `Resync`, `Pong`;
server `Welcome`, `Snapshot`, `Delta`, `Denied`, `AccessChanged`, `AuthExpired`, `Ping`, `Shutdown`, `Reload`.
Transport: a Leptos server function with the Websocket protocol; fallback a plain axum websocket route (same protocol).

## 5. Web

Multi-page SSR (Leptos islands). Sidebar: Live, Voice lines, Reports, Audit, System, then each community → its people
with live dots.

| Route | Content |
|---|---|
| `/` | live wall: one tile per person the bot hears (level sparkline, station, last verdict, lag) + latest violations |
| `/voice-lines` | global voice lines + clip library (upload any format, record in the browser, language tag, transcript, self-check) |
| `/reports` | violations, swear jar, digest preview/send (cursor pages) |
| `/audit` | audit events (cursor pages) |
| `/system` | Status · Global settings · Fluxer & secrets · Web UI address · Models & voices · Storage |
| `/c/:gid` | Overview (calls, tracked people, permission check, add person, invite) · Voice lines · Settings · Reports |
| `/c/:gid/p/:uid/{live,history,evidence,voice-lines,settings}` | person tabs |
| `/setup`, `/login`, `/auth/*` | wizard (setup code → instance → bot token → client secret → owner login), OAuth2 PKCE |

Security: host allowlist (421; IP literals, localhost, the Web UI address setting, extra allowed hosts), tower-http
`CsrfLayer`, Origin check on the websocket, `pb_session = v1.<sid>.<HMAC-SHA256>` cookie (HttpOnly, SameSite=Lax) with
server-side records (owner 12 h, admin 7 d, sensitive actions need a login from the last 15 min), setup code with a
per-IP guessing delay. Uploads: axum multipart, streamed to `tmp/` while hashing, decoded off-thread, original + a
prepared 48 kHz render (−16 LUFS, −1 dBFS peak, fades) stored as blobs, self-check queued (language tag pre-filled from
the classifier's language head). Recording: `MediaRecorder` in an island (web-sys); needs https → optional built-in TLS
(`pb tls generate`, P-256) with upload as the fallback.

## 6. Settings

One `settings!` declaration generates: `Layer` (all optional, unknown keys rejected), `Effective` (each field with its
source), `SettingKey`, `SCHEMA` metadata (kind, scopes, section, owner-only, apply Live|Reconnect|Restart, range for
model/platform limits only, unit, choices, environment name, Fluent ids), and the validators used by TOML, environment,
chat and UI alike. The engine applies changes with an exhaustive `match` on `SettingKey`, so a setting nobody applies does
not compile. Durations are strings (`"20s"`, `"1h"`, `"unlimited"`). Removed: max_per_hour, max_channels, retention
days, recent_clips, step count limit, text length limit, tracked-per-guild limit, all cooldowns.

## 7. Voice lines

Lines: `Warning{label, step}`, `Greeting`, `StrikeNotice`, `Action{mute|unmute|disconnect|timeout}`, `Say{preset}`,
`Name` (the person's spoken name). A slot (global, server or person) holds clips (each tagged with a language or
non-speech) and text per language:

```toml
[voice_lines.warning.profanity.2]
clips = ["sha256:ab…", "sha256:cd…"]
text.de = "{name}, das ist schon das zweite Mal."
text.en = "{name}, that's twice now."
```

Resolution: language L = the person's fixed language, or with "auto" the classifier's language for that sentence,
falling back along the fallback languages; scopes person → server → global → built-in, and inside each scope
label+step → label+any → any+step → any+any; the first slot with a clip in L (or non-speech) wins, else its text in L
through TTS if a voice for L is installed, else the next fallback language, else the built-in text/clip. No clip repeats
twice in a row for the same person and line. A recorded `Name` clip is spliced into the speech around `{name}`.
Placeholders are inserted literally. Pre-rendering calls the same `plan()` for the predicted next violation (count+1,
next step, current channel and server), so the cache key at playback time is the one that was rendered.

## 8. Milestones

M0 skeleton + gates · M1 LiveKit spike · M2 classifier + VAD in Burn with golden tests · M3 Piper on rten with
espeak-ng · M4 pure core · M5 store + importer · M6 Fluxer client + fake · M7 inference runtime · M8 engine end to end ·
M9 live hub · M10 web incl. the two-tab freeze test · M11 container · M12 parity with the Python bot. Each milestone ends
with `cargo xtask ci` green and its own "done when" checks (see the plan).
