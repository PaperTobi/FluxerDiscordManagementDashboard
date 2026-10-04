//! One voice room: joining with the grant, listening only to tracked people's microphones (one track task each),
//! who may hear the bot, and the playback of everything it says.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pb_domain::PlayPurpose;
use pb_domain::{Audience, UserId};
use pb_fluxer_api::{Destination, OutgoingMessage, VoiceGrant};
use pb_i18n::{Locale, text};
use pb_infer::SpeakPriority;
use pb_live_proto::{Activity, PersonDelta};
use pb_policy::Chan;
use pb_segment::{EchoGuard, FlushReason};
use pb_settings::NoSpeakPolicy;
use pb_store_api::{Event, PlayOutcome, PlayRecord};
use pb_voice_api::{
    AudienceSet, AudioOut, ConnectOpts, Identity, Participant, RoomEvent, TrackKey, TrackSource, TransportError,
    VoiceRoom,
};
use pb_voicelines::{Fields, Line};
use tokio::sync::{mpsc, oneshot, watch};

use super::control::ControlMsg;
use super::core::{Core, PlayItem, RoomCmd, RoomHandle};
use super::track::{self, TrackSpec};

/// Silence before a clip (lets the listener's jitter buffer start), and extra when the audience was just narrowed
/// (so the listener has subscribed before it starts).
const LEAD_IN_MS: usize = 120;
const NARROW_PAD_MS: usize = 350;

struct TrackRun {
    user: UserId,
    stop: watch::Sender<Option<FlushReason>>,
    muted: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// What the playback task gets.
enum PlaybackMsg {
    Play(Box<PlayItem>),
    /// Answered once everything queued before is done.
    Drained(oneshot::Sender<()>),
}

/// A microphone subscription that finished (subscribing waits for the server and runs beside the room's loop).
struct Subscribed {
    key: TrackKey,
    user: UserId,
    result: Result<pb_voice_api::AudioIn, TransportError>,
}

/// What a room's loop keeps.
struct Room {
    core: Arc<Core>,
    handle: RoomHandle,
    room: Arc<dyn VoiceRoom>,
    participants: Arc<Mutex<BTreeMap<Identity, Participant>>>,
    tracks: HashMap<TrackKey, TrackRun>,
    /// Subscriptions asked for and not finished yet.
    subscribing: BTreeSet<TrackKey>,
    subscribed: mpsc::UnboundedSender<Subscribed>,
    echo: Arc<Mutex<EchoGuard>>,
    greeted: BTreeSet<UserId>,
    play: mpsc::UnboundedSender<PlaybackMsg>,
    /// The bot stops: nobody new is listened to.
    stopping: bool,
}

/// Starts a room task.
pub fn spawn(
    core: Arc<Core>,
    chan: Chan,
    grant: VoiceGrant,
    control: mpsc::UnboundedSender<ControlMsg>,
    timeout: Duration,
) -> RoomHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = RoomHandle { chan, tx };
    let h2 = handle.clone();
    tokio::spawn(async move { run(core, h2, grant, rx, control, timeout).await });
    handle
}

fn person_of(p: &Identity) -> Option<UserId> {
    p.user()
}

async fn run(
    core: Arc<Core>,
    handle: RoomHandle,
    grant: VoiceGrant,
    mut rx: mpsc::UnboundedReceiver<RoomCmd>,
    control: mpsc::UnboundedSender<ControlMsg>,
    timeout: Duration,
) {
    let chan = handle.chan;
    let (room, mut events) = match core.deps.voice.connect(&grant, ConnectOpts { timeout }).await {
        Ok((room, events)) => (Arc::<dyn VoiceRoom>::from(room), events),
        Err(e) => {
            tracing::warn!(?chan, error = %e, "could not join the voice room");
            let _ = control.send(ControlMsg::RoomDown {
                chan,
                reason: format!("connect failed: {e}"),
            });
            return;
        }
    };
    // The bot's own track; without the Speak permission there is none (the no-speak policy applies).
    let out = match room.publish_voice().await {
        Ok(out) => {
            if let Ok(mut sp) = core.speaking.lock() {
                sp.insert(chan);
            }
            Some(out)
        }
        Err(TransportError::NoSpeakPermission) => {
            tracing::info!(?chan, "no Speak permission in this channel");
            None
        }
        Err(e) => {
            tracing::warn!(?chan, error = %e, "could not publish the bot's voice");
            None
        }
    };
    let _ = control.send(ControlMsg::RoomUp { chan });
    let echo = Arc::new(Mutex::new(EchoGuard::default()));
    let participants: Arc<Mutex<BTreeMap<Identity, Participant>>> = Arc::new(Mutex::new(
        room.participants()
            .into_iter()
            .map(|p| (p.identity.clone(), p))
            .collect(),
    ));
    let (play_tx, play_rx) = mpsc::unbounded_channel::<PlaybackMsg>();
    let playback = tokio::spawn(playback(
        core.clone(),
        chan,
        room.clone(),
        out,
        echo.clone(),
        participants.clone(),
        play_rx,
    ));
    let (subscribed_tx, mut subscribed) = mpsc::unbounded_channel::<Subscribed>();
    let mut r = Room {
        core: core.clone(),
        handle: handle.clone(),
        room: room.clone(),
        participants: participants.clone(),
        tracks: HashMap::new(),
        subscribing: BTreeSet::new(),
        subscribed: subscribed_tx,
        echo: echo.clone(),
        greeted: BTreeSet::new(),
        play: play_tx.clone(),
        stopping: false,
    };
    let mut settings = core.settings.watch();
    r.reconcile().await;
    let (reason, closed) = loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break ("the room closed".to_owned(), None) };
                match ev {
                    RoomEvent::ParticipantJoined(p) => {
                        if let Ok(mut ps) = participants.lock() {
                            ps.insert(p.identity.clone(), p);
                        }
                    }
                    RoomEvent::ParticipantLeft(id) => {
                        if let Ok(mut ps) = participants.lock() {
                            ps.remove(&id);
                        }
                    }
                    RoomEvent::TrackPublished(t) => {
                        if let Ok(mut ps) = participants.lock()
                            && let Some(p) = ps.get_mut(&t.key.participant) {
                                p.audio.retain(|a| a.key != t.key);
                                p.audio.push(t);
                            }
                    }
                    RoomEvent::TrackUnpublished(key) => {
                        if let Ok(mut ps) = participants.lock()
                            && let Some(p) = ps.get_mut(&key.participant) {
                                p.audio.retain(|a| a.key != key);
                            }
                        if let Some(t) = r.tracks.remove(&key) {
                            let _ = t.stop.send(Some(FlushReason::Close));
                            core.set_listening(chan.guild, t.user, false);
                        }
                    }
                    RoomEvent::TrackMuted { key, muted } => {
                        if let Some(t) = r.tracks.get(&key) {
                            let _ = t.muted.send(muted);
                        }
                        if let Ok(mut ps) = participants.lock()
                            && let Some(a) = ps.get_mut(&key.participant).and_then(|p| p.audio.iter_mut().find(|a| a.key == key)) {
                                a.muted = muted;
                            }
                        continue;
                    }
                    RoomEvent::Reconnecting => {
                        tracing::info!(?chan, "the voice room is reconnecting");
                        continue;
                    }
                    RoomEvent::Reconnected => continue,
                    RoomEvent::Disconnected { reason } => break (reason, None),
                }
                r.reconcile().await;
            }
            Some(s) = subscribed.recv() => r.on_subscribed(s),
            cmd = rx.recv() => match cmd {
                None => break (String::new(), None),
                Some(RoomCmd::Close(done)) => break (String::new(), done),
                Some(RoomCmd::Play(item)) => {
                    let _ = play_tx.send(PlaybackMsg::Play(item));
                }
                Some(RoomCmd::Flush(done)) => {
                    r.stopping = true;
                    let tracks: Vec<TrackRun> = r.tracks.drain().map(|(_, t)| t).collect();
                    for t in &tracks {
                        let _ = t.stop.send(Some(FlushReason::End));
                        core.set_listening(chan.guild, t.user, false);
                    }
                    // Awaited beside the loop (the room keeps playing meanwhile).
                    tokio::spawn(async move {
                        for t in tracks {
                            let _ = t.task.await;
                        }
                        let _ = done.send(());
                    });
                }
                Some(RoomCmd::Drain(done)) => {
                    let _ = play_tx.send(PlaybackMsg::Drained(done));
                }
            },
            _ = settings.changed() => r.reconcile().await,
        }
    };
    for (_, t) in r.tracks.drain() {
        let _ = t.stop.send(Some(FlushReason::Close));
        core.set_listening(chan.guild, t.user, false);
    }
    if let Ok(mut sp) = core.speaking.lock() {
        sp.remove(&chan);
    }
    core.mark_guild(chan.guild);
    drop(r);
    drop(play_tx);
    playback.abort();
    room.close().await;
    if !reason.is_empty() {
        tracing::info!(?chan, reason, "left the voice room");
        let _ = control.send(ControlMsg::RoomDown { chan, reason });
    }
    if let Some(done) = closed {
        let _ = done.send(());
    }
}

impl Room {
    /// The tracked people's microphones in the room: (track, person, muted), and the tracked people there.
    fn wanted(&self) -> (BTreeMap<TrackKey, (UserId, bool)>, Vec<Identity>) {
        let tracked = self.core.settings.current().tracked_for(self.handle.chan.guild);
        let bot = self.core.bot();
        let snapshot: Vec<Participant> = self
            .participants
            .lock()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default();
        let mut want = BTreeMap::new();
        let mut tracked_ids = Vec::new();
        for p in &snapshot {
            let Some(u) = person_of(&p.identity) else { continue };
            if Some(u) == bot || !tracked.contains(&u) {
                continue;
            }
            tracked_ids.push(p.identity.clone());
            for a in &p.audio {
                if a.source == TrackSource::Microphone {
                    want.insert(a.key.clone(), (u, a.muted));
                }
            }
        }
        (want, tracked_ids)
    }

    /// Listens to exactly the tracked people's microphones and lets exactly the right people hear the bot. New
    /// subscriptions run in their own tasks: the room keeps playing meanwhile.
    async fn reconcile(&mut self) {
        let chan = self.handle.chan;
        let (want, tracked_ids) = self.wanted();
        // Stop listening to people no longer tracked.
        let gone: Vec<TrackKey> = self.tracks.keys().filter(|k| !want.contains_key(*k)).cloned().collect();
        for k in gone {
            if let Some(t) = self.tracks.remove(&k) {
                let _ = t.stop.send(Some(FlushReason::Close));
                self.core.set_listening(chan.guild, t.user, false);
                let room = self.room.clone();
                tokio::spawn(async move {
                    let _ = room.unsubscribe(&k).await;
                });
                tracing::info!(?chan, user = %t.user, "no longer listening");
            }
        }
        for (key, (user, _)) in want {
            if self.stopping || self.tracks.contains_key(&key) || !self.subscribing.insert(key.clone()) {
                continue;
            }
            let (room, done) = (self.room.clone(), self.subscribed.clone());
            tokio::spawn(async move {
                let result = room.subscribe(&key).await;
                let _ = done.send(Subscribed { key, user, result });
            });
        }
        let audience = default_audience(&self.core, chan, &tracked_ids);
        if let Err(e) = self.room.set_audience(&audience).await {
            tracing::warn!(?chan, error = %e, "could not set who hears the bot");
        }
    }

    /// A subscription finished: listen (when the person is still wanted), show them, greet them.
    fn on_subscribed(&mut self, s: Subscribed) {
        let (core, chan) = (self.core.clone(), self.handle.chan);
        self.subscribing.remove(&s.key);
        let audio = match s.result {
            Ok(audio) => audio,
            Err(e) => {
                tracing::warn!(?chan, user = %s.user, error = %e, "could not subscribe to a microphone");
                return;
            }
        };
        let (want, _) = self.wanted();
        let muted = want.get(&s.key).map(|(_, m)| *m);
        let Some(muted) = muted.filter(|_| !self.stopping && !self.tracks.contains_key(&s.key)) else {
            // No longer wanted while subscribing.
            let room = self.room.clone();
            tokio::spawn(async move {
                let _ = room.unsubscribe(&s.key).await;
            });
            return;
        };
        let user = s.user;
        let (stop_tx, stop_rx) = watch::channel(None);
        let (muted_tx, muted_rx) = watch::channel(muted);
        let spec = TrackSpec {
            chan,
            user,
            room: self.handle.clone(),
            audio,
            echo: self.echo.clone(),
            muted: muted_rx,
            stop: stop_rx,
        };
        let task = tokio::spawn(track::run(core.clone(), spec));
        core.set_listening(chan.guild, user, true);
        tracing::info!(?chan, %user, "listening");
        let eff = core.settings.current().effective(Some(chan.guild), Some(user));
        let gs = core.guilds();
        let who = pb_live_proto::Who {
            user,
            name: gs.name(chan.guild, user),
            avatar: core.avatar_url(
                user,
                gs.person(chan.guild, user).and_then(|p| p.avatar.clone()).as_deref(),
            ),
        };
        core.live.ensure_person(chan.guild, who, super::live::summary(&eff));
        tokio::spawn(super::speak::prerender(core.clone(), chan.guild, user));
        if self.greeted.insert(user) && eff.greet_enabled.value {
            let _ = self.play.send(PlaybackMsg::Play(Box::new(PlayItem {
                line: Line::Greeting,
                person: Some(user),
                audience: Audience::Offender,
                fields: Fields::new(),
                purpose: PlayPurpose::Greeting,
                sentence: None,
                deadline: None,
                by: None,
                text: None,
                heard: None,
                label: None,
                done: None,
            })));
        }
        self.tracks.insert(
            s.key,
            TrackRun {
                user,
                stop: stop_tx,
                muted: muted_tx,
                task,
            },
        );
    }
}

/// Who hears the bot between warnings: everyone (audience "channel"), else the tracked people.
fn default_audience(core: &Core, chan: Chan, tracked: &[Identity]) -> AudienceSet {
    match Audience::from(core.settings.current().effective(Some(chan.guild), None).audience.value) {
        Audience::Channel => AudienceSet::All,
        _ => AudienceSet::Only(tracked.to_vec()),
    }
}

/// Plays queued items one after another (the queue has no limit; an item past its deadline is recorded, not played).
async fn playback(
    core: Arc<Core>,
    chan: Chan,
    room: Arc<dyn VoiceRoom>,
    mut out: Option<Box<dyn AudioOut>>,
    echo: Arc<Mutex<EchoGuard>>,
    participants: Arc<Mutex<BTreeMap<Identity, Participant>>>,
    mut rx: mpsc::UnboundedReceiver<PlaybackMsg>,
) {
    while let Some(msg) = rx.recv().await {
        let mut item = match msg {
            PlaybackMsg::Play(item) => *item,
            PlaybackMsg::Drained(done) => {
                let _ = done.send(());
                continue;
            }
        };
        let started = core.deps.clock.now();
        let id = pb_domain::SentenceId::new();
        let mut record = PlayRecord {
            id,
            guild: chan.guild,
            channel: chan.channel,
            person: item.person,
            purpose: item.purpose,
            sentence: item.sentence,
            line: None,
            clips: Vec::new(),
            text: None,
            lang: None,
            audience: item.audience,
            started,
            dur_ms: 0,
            outcome: PlayOutcome::NothingToSay,
            by: item.by.clone(),
        };
        if item.deadline.is_some_and(|d| core.deps.clock.mono() > d) {
            record.outcome = PlayOutcome::TooLate;
        } else {
            match super::speak::render(
                &core,
                chan.guild,
                Some(chan.channel),
                item.person,
                &item.line,
                item.text.as_ref(),
                item.heard,
                item.label,
                &item.fields,
                SpeakPriority::Live,
            )
            .await
            {
                Err(e) => record.outcome = PlayOutcome::Failed { error: e.to_string() },
                Ok(r) if r.pcm.is_empty() => {
                    record.outcome = PlayOutcome::NothingToSay;
                    record.line = r.line;
                }
                Ok(r) => {
                    record.line.clone_from(&r.line);
                    record.clips.clone_from(&r.clips);
                    record.text.clone_from(&r.text);
                    record.lang.clone_from(&r.lang);
                    match out.as_mut() {
                        Some(o) => {
                            play_one(&core, chan, &room, o, &echo, &participants, &item, &r.pcm, &mut record).await
                        }
                        None => no_speak(&core, chan, &item, r.text.as_deref(), &mut record).await,
                    }
                }
            }
        }
        if let Some(person) = item.person {
            core.live.person(
                chan.guild,
                person,
                PersonDelta::Activity {
                    item: Activity::Play {
                        id: id.0.as_u64_pair().1,
                        kind: item.purpose,
                        at_ms: started.as_millisecond(),
                        text: record.text.clone(),
                        audience: item.audience,
                        ended_ms: Some(core.deps.clock.now().as_millisecond()),
                        ok: Some(record.ok()),
                    },
                },
            );
        }
        core.record(vec![Event::Played(Box::new(record.clone()))]);
        if let Some(done) = item.done.take() {
            let _ = done.send(record);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn play_one(
    core: &Core,
    chan: Chan,
    room: &Arc<dyn VoiceRoom>,
    out: &mut Box<dyn AudioOut>,
    echo: &Arc<Mutex<EchoGuard>>,
    participants: &Arc<Mutex<BTreeMap<Identity, Participant>>>,
    item: &PlayItem,
    pcm: &[i16],
    record: &mut PlayRecord,
) {
    let eff = core.settings.current().effective(Some(chan.guild), item.person);
    let mut samples = pb_audio::from_i16(pcm);
    pb_audio::gain_db(&mut samples, eff.volume_db.value.get() as f32);
    // Who hears it: only the person it is about, the tracked people, or everyone.
    let mut narrowed = false;
    let ids: Vec<Identity> = participants
        .lock()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    let tracked = core.settings.current().tracked_for(chan.guild);
    let tracked_ids: Vec<Identity> = ids
        .iter()
        .filter(|i| person_of(i).is_some_and(|u| tracked.contains(&u)))
        .cloned()
        .collect();
    let audience = match item.audience {
        Audience::Offender => match item.person {
            Some(u) => {
                narrowed = true;
                AudienceSet::Only(ids.iter().filter(|i| person_of(i) == Some(u)).cloned().collect())
            }
            None => AudienceSet::Only(tracked_ids.clone()),
        },
        Audience::Tracked => AudienceSet::Only(tracked_ids.clone()),
        Audience::Channel => {
            narrowed = Audience::from(core.settings.current().effective(Some(chan.guild), None).audience.value)
                != Audience::Channel;
            AudienceSet::All
        }
    };
    if let Err(e) = room.set_audience(&audience).await {
        tracing::warn!(?chan, error = %e, "could not set who hears the bot");
    }
    let lead = LEAD_IN_MS + if narrowed { NARROW_PAD_MS } else { 0 };
    let mut full = vec![0i16; lead * pb_audio::PLAY_RATE as usize / 1000];
    full.extend(pb_audio::to_i16(&samples));
    if let Ok(mut e) = echo.lock() {
        e.begin(core.deps.clock.mono());
    }
    let r = out.play(&full).await;
    if let Ok(mut e) = echo.lock() {
        e.end(core.deps.clock.mono());
    }
    {
        record.dur_ms = (full.len() as u64 * 1000 / u64::from(pb_audio::PLAY_RATE)) as u32;
    }
    record.outcome = match r {
        Ok(()) => PlayOutcome::Played,
        Err(e) => PlayOutcome::Failed { error: e.to_string() },
    };
    let back = default_audience(core, chan, &tracked_ids);
    if back != audience {
        let _ = room.set_audience(&back).await;
    }
}

/// Without the Speak permission: write it into the voice channel's chat, or only record it.
async fn no_speak(core: &Core, chan: Chan, item: &PlayItem, said: Option<&str>, record: &mut PlayRecord) {
    let eff = core.settings.current().effective(Some(chan.guild), item.person);
    if eff.no_speak_policy.value != NoSpeakPolicy::Text {
        record.outcome = PlayOutcome::NotSpoken;
        return;
    }
    let (Some(ctl), Some(said)) = (core.ctl(), said) else {
        record.outcome = PlayOutcome::NotSpoken;
        return;
    };
    let loc = Locale::for_lang(
        &core
            .settings
            .current()
            .effective(Some(chan.guild), None)
            .chat_language
            .value,
    );
    let user = item.person.map_or_else(String::new, |u| format!("<@{u}>"));
    let content = text(loc, "no-speak", &[("user", user.into()), ("text", said.into())]);
    let ping = item.person.into_iter().collect();
    match ctl
        .send(
            Destination::Channel(chan.channel),
            OutgoingMessage {
                content,
                reply_to: None,
                ping,
                files: vec![],
            },
        )
        .await
    {
        Ok(_) => record.outcome = PlayOutcome::Written,
        Err(e) => record.outcome = PlayOutcome::Failed { error: e.to_string() },
    }
}
