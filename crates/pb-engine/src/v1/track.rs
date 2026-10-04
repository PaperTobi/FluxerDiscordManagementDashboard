//! One microphone: frames → voice activity → sentences → the classifier → the moderation actor. Also the live levels
//! and each sentence's way along the conveyor.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use pb_domain::{GuildId, SentenceId, UserId};
use pb_infer::{InferError, Job, Priority, Scored};
use pb_live_proto::{CutWhy, DropWhy, LevelFrame, LevelRun, SentenceCard, Stamps};
use pb_models_api::FRAME;
use pb_policy::Chan;
use pb_segment::{
    CutReason, EchoGuard, Event as SegEvent, FlushReason, FrameAssembler, PcmRing, SegCfg, Segmenter,
    keep_before_playback,
};
use pb_store_api::CutCause;
use pb_voice_api::{AudioIn, LISTEN_RATE};
use tokio::sync::{mpsc, watch};

use super::core::{Core, RoomHandle};
use super::moderation::{Heard, ModMsg};

/// Frames per level update (8 × 32 ms ≈ 4 updates per second).
const LEVEL_BATCH: usize = 8;
/// Frames stop arriving for this long while a sentence is open: close it instead of waiting.
const STALL_S: f64 = 1.0;
/// Of a sentence that overlaps the bot's own playback, at least this much before the playback must remain.
const MIN_KEEP_S: f64 = 0.6;

/// The segmenter settings for a person.
pub fn segcfg(eff: &pb_settings::Effective) -> SegCfg {
    SegCfg {
        end_silence_ms: eff.end_silence.value.get().millis() as u32,
        max_clip_s: eff.max_sentence.value.get().secs(),
        min_voiced_ms: eff.min_voiced.value.get().millis() as u32,
        ..SegCfg::default()
    }
    .normalized()
}

fn cut_why(r: CutReason) -> (CutWhy, CutCause) {
    match r {
        CutReason::Silence => (CutWhy::Pause, CutCause::Pause),
        CutReason::MaxSoft | CutReason::MaxHard => (CutWhy::MaxLength, CutCause::MaxLength),
        CutReason::Mute => (CutWhy::Muted, CutCause::Muted),
        CutReason::Disconnect | CutReason::Close => (CutWhy::Left, CutCause::Left),
        CutReason::Stall => (CutWhy::StreamStalled, CutCause::StreamStalled),
        CutReason::End => (CutWhy::Shutdown, CutCause::Shutdown),
    }
}

/// What a track task needs.
pub struct TrackSpec {
    pub chan: Chan,
    pub user: UserId,
    pub room: RoomHandle,
    pub audio: AudioIn,
    pub echo: watch::Receiver<EchoGuard>,
    pub muted: watch::Receiver<bool>,
    /// Set when listening ends, with why (`Close`: the person left or is no longer tracked; `End`: the bot stops).
    pub stop: watch::Receiver<Option<FlushReason>>,
}

struct Open {
    id: SentenceId,
    no: u32,
    stamps: Stamps,
}

fn ms(t: Timestamp) -> i64 {
    t.as_millisecond()
}

/// Runs until the track ends or `stop` is set; open speech is flushed into a last sentence.
pub async fn run(core: Arc<Core>, mut spec: TrackSpec) {
    let (g, u) = (spec.chan.guild, spec.user);
    let mut vad = core.deps.inference.vad_stream();
    let mut asm = FrameAssembler::new();
    let mut ring = PcmRing::new(0);
    let eff = core.settings.current().effective(Some(g), Some(u));
    let mut cfg = segcfg(&eff);
    let mut next_id = 0u64;
    let mut seg = Segmenter::new(
        cfg.clone(),
        move || {
            next_id += 1;
            next_id
        },
        0,
    );
    let mut settings = core.settings.watch();
    let mut k: i64 = 0;
    // Wall and monotonic time of the newest sample (sample index = ring end).
    let mut last_mono = core.deps.clock.mono();
    let mut last_feed = last_mono;
    let mut open: std::collections::HashMap<u64, Open> = std::collections::HashMap::new();
    let mut levels: Vec<LevelFrame> = Vec::with_capacity(LEVEL_BATCH);
    let mut reconfigure = false;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let (jobs, queued) = mpsc::unbounded_channel();
    let scorer = core.sup.spawn_task("scorer", score(core.clone(), g, u, queued));
    loop {
        let mut events: Vec<SegEvent> = Vec::new();
        tokio::select! {
            chunk = spec.audio.recv() => {
                let Some(chunk) = chunk else {
                    events.extend(seg.flush(i64::try_from(ring.end()).unwrap_or(i64::MAX), FlushReason::Disconnect));
                    handle(&core, &spec, &jobs, &mut ring, &mut open, events, last_mono);
                    break;
                };
                last_mono = core.deps.clock.mono();
                last_feed = last_mono;
                let frames = asm.push(&chunk.samples);
                if frames.is_empty() {
                    continue;
                }
                let floats: Vec<[f32; FRAME]> = frames.iter().map(|f| std::array::from_fn(|i| f32::from(f[i]) / 32768.0)).collect();
                for f in &frames {
                    ring.append(f);
                }
                let Ok(probs) = vad.step(floats.clone()).await else { break };
                let watched = core.live.watched(g, u);
                for (i, p) in probs.iter().enumerate() {
                    events.extend(seg.push(k, f64::from(*p)));
                    k += 1;
                    if watched {
                        levels.push(LevelFrame::new(*p, pb_audio::rms_dbfs(&floats[i])));
                        if levels.len() >= LEVEL_BATCH {
                            let now = core.deps.clock.now();
                            core.live.levels(g, u, LevelRun { end_ms: ms(now), frame_ms: 32, frames: std::mem::take(&mut levels) });
                        }
                    }
                }
            }
            changed = spec.muted.changed() => {
                if changed.is_err() {
                    continue;
                }
                if *spec.muted.borrow() {
                    events.extend(seg.flush(i64::try_from(ring.end()).unwrap_or(i64::MAX), FlushReason::Mute));
                } else {
                    // A long pause: the recurrent state is stale.
                    vad.reset();
                }
            }
            _ = settings.changed() => {
                let eff = core.settings.current().effective(Some(g), Some(u));
                let new = segcfg(&eff);
                if new != cfg {
                    cfg = new;
                    reconfigure = true;
                }
            }
            _ = tick.tick() => {
                let now = core.deps.clock.mono();
                if now - last_feed > STALL_S && seg.state() != pb_segment::SegState::Idle {
                    events.extend(seg.flush(i64::try_from(ring.end()).unwrap_or(i64::MAX), FlushReason::Stall));
                }
            }
            _ = spec.stop.changed() => {
                let why = (*spec.stop.borrow()).unwrap_or(FlushReason::Close);
                events.extend(seg.flush(i64::try_from(ring.end()).unwrap_or(i64::MAX), why));
                handle(&core, &spec, &jobs, &mut ring, &mut open, events, last_mono);
                break;
            }
        }
        handle(&core, &spec, &jobs, &mut ring, &mut open, events, last_mono);
        // New timing settings apply at the next pause (a sentence is never cut by a settings change).
        if reconfigure && seg.state() == pb_segment::SegState::Idle {
            seg.reconfigure(cfg.clone());
            reconfigure = false;
        }
        let keep = u64::try_from(seg.earliest_needed().max(0)).unwrap_or(0);
        ring.forget_before(keep.saturating_sub(u64::from(LISTEN_RATE)));
    }
    // The sentences cut so far are still scored and decided.
    drop(jobs);
    let _ = scorer.await;
}

/// A cut sentence on its way to the classifier.
struct ScoreJob {
    pcm: Arc<[f32]>,
    /// The conveyor card so far.
    card: SentenceCard,
    /// When a verdict stops being useful for a warning (it is still scored and recorded after that).
    deadline: Option<std::time::Instant>,
    heard: Box<dyn FnOnce(Result<Scored, InferError>, Stamps) -> Heard + Send>,
}

/// Scores a track's sentences one after another and hands each to moderation, in the order they were said.
async fn score(core: Arc<Core>, g: GuildId, u: UserId, mut jobs: mpsc::UnboundedReceiver<ScoreJob>) {
    while let Some(j) = jobs.recv().await {
        let mut stamps = j.card.stamps;
        stamps.scoring = Some(ms(core.deps.clock.now()));
        core.live.sentence(g, u, &SentenceCard { stamps, ..j.card });
        let job = Job {
            priority: Priority::Live,
            deadline: j.deadline,
            flow: Some(u.get()),
        };
        let scored = core.deps.inference.classify(j.pcm, job).await;
        let _ = core.moderation.send(ModMsg::Heard(Box::new((j.heard)(scored, stamps))));
    }
}

fn handle(
    core: &Arc<Core>,
    spec: &TrackSpec,
    jobs: &mpsc::UnboundedSender<ScoreJob>,
    ring: &mut PcmRing,
    open: &mut std::collections::HashMap<u64, Open>,
    events: Vec<SegEvent>,
    last_mono: f64,
) {
    let (g, u) = (spec.chan.guild, spec.user);
    let mono_of = |sample: i64, ring_end: u64| -> f64 {
        last_mono - (i64::try_from(ring_end).unwrap_or(i64::MAX) - sample).max(0) as f64 / f64::from(LISTEN_RATE)
    };
    for ev in events {
        match ev {
            SegEvent::Open(o) => {
                let now = core.deps.clock.now();
                let started =
                    ms(now) - ((i64::try_from(ring.end()).unwrap_or(0) - o.s0).max(0) * 1000 / i64::from(LISTEN_RATE));
                let card = Open {
                    id: SentenceId::new(),
                    no: core.next_sentence_no(g, u),
                    stamps: Stamps {
                        opened: started,
                        ..Stamps::default()
                    },
                };
                core.live.sentence(g, u, &card_of(&card, None));
                open.insert(o.id, card);
            }
            SegEvent::Drop(d) => {
                if let Some(mut c) = open.remove(&d.id) {
                    c.stamps.dropped = Some(ms(core.deps.clock.now()));
                    core.live.sentence(
                        g,
                        u,
                        &SentenceCard {
                            dropped: Some(DropWhy::TooLittleSpeech),
                            ..card_of(&c, None)
                        },
                    );
                }
            }
            SegEvent::Blip { .. } => {}
            SegEvent::Cut(c) => {
                let mut card = open.remove(&c.id).unwrap_or_else(|| Open {
                    id: SentenceId::new(),
                    no: core.next_sentence_no(g, u),
                    stamps: Stamps {
                        opened: ms(core.deps.clock.now()),
                        ..Stamps::default()
                    },
                });
                let now = core.deps.clock.now();
                card.stamps.cut = Some(ms(now));
                let (why, cause) = cut_why(c.reason);
                let s0 = u64::try_from(c.s0.max(0)).unwrap_or(0);
                let s1 = u64::try_from(c.s1.max(0)).unwrap_or(0);
                let mut pcm = ring.slice(s0, s1);
                let (t0, t1) = (mono_of(c.s0, ring.end()), mono_of(c.s1, ring.end()));
                // The bot's own voice picked up by this microphone is never scored as theirs.
                let guard = spec.echo.borrow().clone();
                if guard.overlaps(t0, t1) {
                    match keep_before_playback(t0, guard.started(), LISTEN_RATE, MIN_KEEP_S) {
                        Some(n) => pcm.truncate(n),
                        None => {
                            card.stamps.dropped = Some(ms(now));
                            core.live.sentence(
                                g,
                                u,
                                &SentenceCard {
                                    cut: Some(why),
                                    dropped: Some(DropWhy::OwnPlayback),
                                    ..card_of(&card, None)
                                },
                            );
                            continue;
                        }
                    }
                }
                if pcm.is_empty() {
                    continue;
                }
                let x: Arc<[f32]> = pb_audio::from_i16(&pcm).into();
                let level_db = pb_audio::rms_dbfs(&x);
                let dur_ms = (x.len() as u64 * 1000 / u64::from(LISTEN_RATE)) as u32;
                card.stamps.queued = Some(ms(core.deps.clock.now()));
                let partial = SentenceCard {
                    cut: Some(why),
                    dur_ms: Some(dur_ms),
                    level_db: Some(level_db),
                    ..card_of(&card, None)
                };
                core.live.sentence(g, u, &partial);
                let started = Timestamp::from_millisecond(card.stamps.opened).unwrap_or(now);
                let cut_mono = core.deps.clock.mono();
                let room = spec.room.clone();
                let chan = spec.chan;
                let deadline = core
                    .settings
                    .current()
                    .effective(Some(g), Some(u))
                    .max_reaction_delay
                    .value
                    .value()
                    .map(|d| std::time::Instant::now() + Duration::from_secs_f64(d.get().secs()));
                let (id, no, pcm) = (card.id, card.no, x.clone());
                let _ = jobs.send(ScoreJob {
                    pcm: x,
                    card: SentenceCard {
                        stamps: card.stamps,
                        ..partial
                    },
                    deadline,
                    heard: Box::new(move |scored, stamps| Heard {
                        id,
                        no,
                        chan,
                        user: u,
                        room,
                        started,
                        dur_ms,
                        level_db,
                        cut: cause,
                        cut_why: why,
                        pcm,
                        scored,
                        cut_mono,
                        stamps,
                    }),
                });
            }
        }
    }
}

/// A conveyor card for a sentence so far.
fn card_of(c: &Open, cut: Option<CutWhy>) -> SentenceCard {
    SentenceCard {
        id: c.id,
        no: c.no,
        stamps: c.stamps,
        dur_ms: None,
        level_db: None,
        cut,
        dropped: None,
        error: None,
        verdict: None,
        decision: None,
    }
}
