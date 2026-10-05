//! The whole bot, black box, on stand-ins (see `common`): what a person in a call experiences, what is recorded and what
//! is reported. These pin the engine's behaviour through its public API while its internals are rebuilt
//! (docs/proposals/0004-engine-actors.md); the scenarios with real models and LiveKit are in `crates/pb/tests`.
//!
//! A test that describes behaviour the engine does not have yet is ignored with the migration step that brings it.
#![allow(clippy::disallowed_methods)] // the tests drive the engine from outside (its own tasks are supervised)
#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use std::time::Duration;

use common::*;
use pb_domain::{ActionKind, ActionOutcome, Audience, ChannelId, ClfLang, GuildId, Label, PlayPurpose, Scope, UserId};
use pb_engine::{Connection, EngineError, SayWhat, VoiceError};
use pb_settings::SettingKey;
use pb_store_api::{
    Actor, CutCause, DecisionRecord, Event, Index, MessagePurpose, PlayOutcome, SentenceRecord, Stopped, Via,
};
use pb_testkit::models::{BENIGN_HZ, BeepTts, Gate, PROFANE_HZ, tone};
use pb_voice_api::{AudienceSet, LISTEN_RATE};
use serde_json::json;

fn warned(s: &SentenceRecord) -> bool {
    matches!(s.decision, DecisionRecord::Warn { .. })
}

/// Where an event is in the log.
fn position(events: &[Event], f: impl Fn(&Event) -> bool) -> usize {
    events.iter().position(f).expect("the event is in the log")
}

/// Loud samples in what the bot played.
fn loud(pcm: &[i16]) -> usize {
    pcm.iter().filter(|s| s.unsigned_abs() > 500).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_profane_sentence_is_warned_in_the_call_recorded_and_reported() {
    let rig = Rig::start(Setup {
        settings: Box::new(|t| set(t, Scope::Global, SettingKey::LeaveGrace, json!(0.3))),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins().await;
    mic.say(&sentence(PROFANE_HZ)).await;

    let s = rig.wait_sentence("a warned sentence", warned).await;
    assert_eq!(
        (s.guild, s.channel, s.user),
        (GuildId(G), ChannelId(VOICE), UserId(ALICE))
    );
    assert_eq!(s.cut, CutCause::Pause);
    assert_eq!(s.flagged, [Label::Profanity]);
    assert!(
        matches!(
            s.decision,
            DecisionRecord::Warn {
                label: Label::Profanity,
                step: 1,
                count: 1,
                ..
            }
        ),
        "{:?}",
        s.decision
    );
    assert_eq!(s.language, ClfLang::En);
    assert!(s.jar, "a violation goes into the swear jar");
    assert!(s.audio.is_some(), "a flagged sentence's recording is kept");

    // The warning, in Alice's language, to the tracked people in the call.
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!(p.outcome, PlayOutcome::Played);
    assert_eq!(p.purpose, PlayPurpose::Warning);
    assert_eq!(p.person, Some(UserId(ALICE)));
    assert_eq!(p.audience, Audience::Tracked);
    assert_eq!(p.lang.as_ref().map(ToString::to_string).as_deref(), Some("en"));
    assert!(p.text.as_deref().is_some_and(|t| t.contains("Alice")), "{p:?}");
    let items = rig.voice.played_items(GuildId(G), ChannelId(VOICE));
    assert_eq!(items.len(), 1);
    assert!(loud(&items[0].samples) > 4800, "the warning is audible");
    assert!(
        matches!(&items[0].audience, Some(AudienceSet::Only(who)) if who.len() == 1 && who[0].user() == Some(UserId(ALICE))),
        "only Alice hears it: {:?}",
        items[0].audience
    );
    let events = rig.events().await;
    let recorded = position(&events, |e| matches!(e, Event::Sentence(x) if x.id == s.id));
    assert!(
        recorded < position(&events, |e| matches!(e, Event::Played(x) if x.id == p.id)),
        "the sentence is recorded before its warning"
    );
    assert!(
        matches!(&events[recorded - 1], Event::BlobAdded(b) if s.audio.as_ref() == Some(&b.hash)),
        "the recording is added right before its sentence"
    );

    // The search index has it, and the swear jar counts it.
    rig.index.caught_up(rig.log.head().unwrap().seq).await;
    let row = rig
        .index
        .sentence(s.id)
        .await
        .unwrap()
        .expect("the index has the sentence");
    assert_eq!(row.record.decision, s.decision);
    assert_eq!(row.record.audio, s.audio);
    let jar = rig.index.jar(Some(GuildId(G))).await.unwrap();
    assert!(jar.iter().any(|j| j.user == UserId(ALICE) && j.count == 1), "{jar:?}");

    // One post in the mod log, and the owner's direct message with the recording.
    rig.wait_event(
        "the mod log post",
        10,
        |e| matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::Modlog { sentence: s.id } && m.ok),
    )
    .await;
    rig.wait_event("the owner's direct message", 10, |e| {
        matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::OwnerDm { sentence: s.id } && m.ok && m.with_audio)
    })
    .await;
    let events = rig.events().await;
    assert!(
        recorded
            < position(&events, |e| {
                matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::Modlog { sentence: s.id })
            }),
        "the sentence is recorded before its report"
    );
    let modlog: Vec<_> = rig.fake.sent().into_iter().filter(|m| m.channel == TEXT).collect();
    assert_eq!(modlog.len(), 1);
    assert!(modlog[0].content().contains(&format!("<@{ALICE}>")), "{:?}", modlog[0]);
    let dm = rig.fake.dm_channel(OWNER).expect("a direct message to the owner");
    assert!(rig.fake.sent().iter().any(|m| m.channel == dm && m.files.len() == 1));

    // Alice leaves the call; the bot leaves too, after the leave grace.
    rig.fake.voice_leave(&rig.alice_connection());
    mic.leave();
    wait("the bot leaves Fluxer's voice channel", 10, || {
        rig.fake.bot_connections().is_empty()
    })
    .await;
    wait("the bot leaves the room", 10, || {
        !rig.voice.has_bot(GuildId(G), ChannelId(VOICE))
    })
    .await;
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_harmless_sentence_is_recorded_without_a_warning() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    mic.say(&sentence(BENIGN_HZ)).await;
    let benign = rig.wait_sentence("a scored sentence", |_| true).await;
    assert_eq!(benign.decision, DecisionRecord::NothingFlagged);
    assert!(benign.flagged.is_empty());
    assert!(!benign.jar);
    assert!(benign.audio.is_none(), "an unflagged sentence keeps no recording");

    // A profane sentence after it is the barrier: each person's sentences are decided and voiced in order.
    mic.say(&sentence(PROFANE_HZ)).await;
    let profane = rig.wait_sentence("the next sentence", |s| s.id != benign.id).await;
    assert!(warned(&profane), "{:?}", profane.decision);
    rig.wait_played("its warning", |p| p.sentence == Some(profane.id)).await;
    rig.wait_event(
        "its mod log post",
        10,
        |e| matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::Modlog { sentence: profane.id }),
    )
    .await;
    assert_eq!(rig.played().await.len(), 1, "only the profane sentence was answered");
    assert_eq!(rig.voice.played_items(GuildId(G), ChannelId(VOICE)).len(), 1);
    assert!(!rig.events().await.iter().any(|e| {
        matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::Modlog { sentence: benign.id })
    }));
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strikes_then_a_mute_that_is_lifted() {
    let rig = Rig::start(Setup {
        settings: Box::new(|t| {
            let mut c = set(t, community(), SettingKey::Strikes, json!(2));
            c.extend(set(t, community(), SettingKey::StrikeNotice, json!(true)));
            c.extend(set(t, community(), SettingKey::ActionsEnabled, json!(true)));
            c.extend(set(
                t,
                community(),
                SettingKey::Escalation,
                json!([{"from": 1, "action": "mute", "duration": 2}]),
            ));
            c
        }),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins().await;
    mic.say(&sentence(PROFANE_HZ)).await;
    let first = rig
        .wait_sentence("a strike", |s| {
            s.decision == DecisionRecord::Strike { strike: 1, of: 2 }
        })
        .await;
    let notice = rig
        .wait_played("the strike notice", |p| p.purpose == PlayPurpose::StrikeNotice)
        .await;
    assert_eq!(notice.sentence, Some(first.id));
    assert_eq!(notice.outcome, PlayOutcome::Played);
    echo_fades().await;
    mic.say(&sentence(PROFANE_HZ)).await;
    let second = rig.wait_sentence("the violation", warned).await;
    assert!(
        matches!(second.decision, DecisionRecord::Warn { step: 1, count: 1, .. }),
        "{:?}",
        second.decision
    );

    let mute = rig.wait_action("the mute", |a| a.undoes.is_none()).await;
    assert_eq!((mute.kind, &mute.outcome), (ActionKind::Mute, &ActionOutcome::Done));
    assert_eq!((mute.sentence, mute.secs), (Some(second.id), Some(2)));
    assert!(mute.undo_at.is_some());
    assert!(
        rig.fake
            .patches()
            .iter()
            .any(|p| p.user == ALICE && p.body["mute"] == true),
        "Fluxer was asked to mute: {:?}",
        rig.fake.patches()
    );

    let unmute = rig
        .wait_action("the mute is lifted", |a| a.undoes == Some(mute.id))
        .await;
    assert_eq!(
        (unmute.kind, &unmute.outcome),
        (ActionKind::Unmute, &ActionOutcome::Done)
    );
    assert!(
        rig.fake
            .patches()
            .iter()
            .any(|p| p.user == ALICE && p.body["mute"] == false)
    );
    let events = rig.events().await;
    let at = |id| position(&events, |e| matches!(e, Event::Action(a) if a.id == id));
    assert!(position(&events, |e| matches!(e, Event::Sentence(s) if s.id == second.id)) < at(mute.id));
    assert!(at(mute.id) < at(unmute.id), "the mute is recorded before its undo");
    rig.index.caught_up(rig.log.head().unwrap().seq).await;
    assert!(rig.index.pending_undos().await.unwrap().is_empty());
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_speak_it_writes_in_the_channel_chat_and_greets() {
    let rig = Rig::start(Setup {
        deny_speak: true,
        settings: Box::new(|t| set(t, community(), SettingKey::GreetEnabled, json!(true))),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins().await;
    let greeting = rig
        .wait_played("the greeting", |p| p.purpose == PlayPurpose::Greeting)
        .await;
    assert_eq!(greeting.outcome, PlayOutcome::Written);
    let greeted = greeting.text.expect("the greeting's text");
    assert!(
        rig.fake.sent().iter().any(|m| m.channel == VOICE
            && m.content().contains(&format!("<@{ALICE}>"))
            && m.content().contains(&greeted)),
        "the greeting is in the voice channel's chat: {:?}",
        rig.fake.sent()
    );

    mic.say(&sentence(PROFANE_HZ)).await;
    let s = rig.wait_sentence("a warned sentence", warned).await;
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!((p.purpose, &p.outcome), (PlayPurpose::Warning, &PlayOutcome::Written));
    let said = p.text.expect("the warning's text");
    assert!(
        rig.fake
            .sent()
            .iter()
            .any(|m| m.channel == VOICE && m.content().contains(&said))
    );
    assert!(
        rig.voice.played_items(GuildId(G), ChannelId(VOICE)).is_empty(),
        "nothing was said aloud"
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_resume_keeps_the_bot_in_the_call() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    let before = rig.fake.bot_connections();
    let ops = rig.fake.voice_updates().len();
    rig.fake.drop_connections(4000);
    wait("the bot notices the drop", 5, || {
        matches!(rig.engine.connection(), Connection::Retrying(_))
    })
    .await;
    wait("the session is resumed", 10, || {
        rig.engine.connection() == Connection::Ready
    })
    .await;
    assert_eq!(rig.fake.bot_connections(), before, "the same voice connection");
    assert_eq!(rig.fake.voice_updates().len(), ops, "no voice-state update was needed");
    assert!(rig.voice.has_bot(GuildId(G), ChannelId(VOICE)) && mic.heard());

    mic.say(&sentence(PROFANE_HZ)).await;
    let s = rig.wait_sentence("a warned sentence", warned).await;
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!(p.outcome, PlayOutcome::Played);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_gateway_session_rejoins_the_call() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    let before = rig.fake.bot_connections()[0].0.clone();
    // The session cannot be resumed: the client identifies again and gets a new session.
    rig.fake.forget_sessions();
    wait("the bot joins again with a new connection", 15, || {
        let conns = rig.fake.bot_connections();
        conns.len() == 1 && conns[0].0 != before && !conns[0].3
    })
    .await;
    wait("the bot listens to Alice again", 10, || mic.heard()).await;
    assert_eq!(rig.engine.connection(), Connection::Ready);

    mic.say(&sentence(PROFANE_HZ)).await;
    let s = rig.wait_sentence("a warned sentence", warned).await;
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!(p.outcome, PlayOutcome::Played);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dropped_voice_room_is_joined_again() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    let before = rig.fake.bot_connections()[0].0.clone();
    rig.voice
        .disconnect(GuildId(G), ChannelId(VOICE), "the voice server restarted");
    wait("the bot leaves and joins again with a new connection", 10, || {
        let conns = rig.fake.bot_connections();
        conns.len() == 1 && conns[0].0 != before && !conns[0].3
    })
    .await;
    wait("the bot listens to Alice again", 10, || mic.heard()).await;

    mic.say(&sentence(PROFANE_HZ)).await;
    let s = rig.wait_sentence("a warned sentence", warned).await;
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!(p.outcome, PlayOutcome::Played);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_commands_in_german() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    let chat = rig.fake.say(G, TEXT, OWNER, &[], "!pb set chat_language de");
    wait("the chat language is set", 10, || rig.reacted(chat, "✅")).await;

    let status = rig.fake.say(G, TEXT, OWNER, &[], "!pb status");
    wait("the status, in German", 10, || {
        rig.replies_to(status)
            .iter()
            .any(|m| m.content().contains("Status in dieser Community") && m.content().contains("Im Sprachkanal"))
    })
    .await;

    let language = rig
        .fake
        .say(G, TEXT, OWNER, &[], &format!("!pb set language de <@{ALICE}>"));
    wait("Alice's language is set, answered in German", 10, || {
        rig.reacted(language, "✅") && rig.replies_to(language).iter().any(|m| m.content().contains("jetzt"))
    })
    .await;

    // Alice may not change settings (she is tracked here).
    let denied = rig.fake.say(G, TEXT, ALICE, &[], "!pb pause");
    wait("the refusal, in German", 10, || {
        rig.reacted(denied, "🚫")
            && rig
                .replies_to(denied)
                .iter()
                .any(|m| m.content().contains("Nur Admins"))
    })
    .await;

    // Her next warning is in German, in the German voice.
    mic.say(&sentence(PROFANE_HZ)).await;
    let s = rig.wait_sentence("a warned sentence", warned).await;
    let p = rig.wait_played("the warning", |p| p.sentence == Some(s.id)).await;
    assert_eq!(p.outcome, PlayOutcome::Played);
    assert_eq!(p.lang.as_ref().map(ToString::to_string).as_deref(), Some("de"), "{p:?}");
    let said = p.text.expect("the warning's text");
    assert!(said.contains("achte auf deine Wortwahl"), "{said}");
    assert!(
        rig.tts.log().iter().any(|r| r.voice == BeepTts::DE && r.text == said),
        "{:?}",
        rig.tts.log()
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn say_now_speaks_in_the_call() {
    let rig = Rig::start(Setup::default()).await;
    let _mic = rig.alice_joins().await;
    let by = Actor {
        user: Some(UserId(OWNER)),
        name: Some("The Owner".into()),
        via: Via::Web,
    };
    let what = || SayWhat::Text {
        text: "Please mind the rules.".into(),
        lang: None,
    };
    let rec = rig
        .engine
        .say_to(GuildId(G), UserId(ALICE), what(), by.clone())
        .await
        .unwrap();
    assert_eq!((rec.purpose, &rec.outcome), (PlayPurpose::Say, &PlayOutcome::Played));
    assert_eq!(rec.person, Some(UserId(ALICE)));
    assert_eq!(rec.text.as_deref(), Some("Please mind the rules."));
    assert_eq!(rec.lang.as_ref().map(ToString::to_string).as_deref(), Some("en"));
    assert_eq!(rec.by.as_ref(), Some(&by));
    assert!(rec.dur_ms > 0);
    let items = rig.voice.played_items(GuildId(G), ChannelId(VOICE));
    assert_eq!(items.len(), 1);
    assert!(loud(&items[0].samples) > 0);
    assert!(
        rig.tts
            .log()
            .iter()
            .any(|r| r.voice == BeepTts::EN && r.text == "Please mind the rules.")
    );
    rig.wait_played("the record of it", |p| p.id == rec.id).await;

    // Someone who is not in a call cannot be spoken to.
    assert_eq!(
        rig.engine.say_to(GuildId(G), UserId(OWNER), what(), by).await,
        Err(EngineError::NotInCall)
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_voice_made_from_a_sample_speaks_its_kind_of_line_until_removed() {
    let rig = Rig::start(Setup::default()).await;
    let _mic = rig.alice_joins().await;
    let by = Actor {
        user: Some(UserId(OWNER)),
        name: Some("The Owner".into()),
        via: Via::Web,
    };
    // A sample at 300 Hz: the beep model's voice made from it beeps at 300 Hz.
    let staged = rig.engine.upload_dir().join("sample.wav");
    std::fs::write(
        &staged,
        pb_audio::wav16(&tone(300.0, 1.5, LISTEN_RATE, 0.3), LISTEN_RATE),
    )
    .unwrap();
    let rec = rig
        .engine
        .add_voice(
            staged,
            Some("wav".into()),
            "Low one".into(),
            "beep".into(),
            None,
            by.clone(),
        )
        .await
        .unwrap();
    assert_eq!(rec.voice_id(), "beep:low-one");
    assert!(rig.engine.voices().iter().any(|v| v.full_id() == "beep:low-one"));
    assert_eq!(rig.engine.library_voices(), std::slice::from_ref(&rec));
    rig.wait_event(
        "the voice is recorded",
        5,
        |e| matches!(e, Event::VoiceSaved(v) if v.id == "low-one"),
    )
    .await;
    // Cloning needs a model that can.
    let staged = rig.engine.upload_dir().join("again.wav");
    std::fs::write(
        &staged,
        pb_audio::wav16(&tone(300.0, 1.0, LISTEN_RATE, 0.3), LISTEN_RATE),
    )
    .unwrap();
    assert_eq!(
        rig.engine
            .add_voice(staged, Some("wav".into()), "X".into(), "piper".into(), None, by.clone())
            .await,
        Err(EngineError::Voice(VoiceError::NoModel("piper".into())))
    );

    // "Say now" lines speak in it; warnings keep the voice for their language.
    rig.engine
        .settings()
        .change(by.clone(), |t| {
            Ok(t.set(
                Scope::Global,
                SettingKey::LineVoices,
                json!({"say": "beep:low-one"}),
                true,
            )?
            .into_iter()
            .collect())
        })
        .await
        .unwrap();
    let say = |text: &'static str| SayWhat::Text {
        text: text.into(),
        lang: None,
    };
    rig.engine
        .say_to(GuildId(G), UserId(ALICE), say("In the new voice."), by.clone())
        .await
        .unwrap();
    assert!(
        rig.tts
            .log()
            .iter()
            .any(|r| r.voice == "low-one" && r.text == "In the new voice.")
    );

    // A new name keeps the id; once removed, the line speaks in the voice for its language again.
    let renamed = rig
        .engine
        .rename_voice("beep:low-one", "Deep".into(), by.clone())
        .await
        .unwrap();
    assert_eq!(
        (renamed.voice_id().as_str(), renamed.name.as_str()),
        ("beep:low-one", "Deep")
    );
    rig.engine.remove_voice("beep:low-one", by.clone()).await.unwrap();
    assert!(rig.engine.library_voices().is_empty());
    assert!(!rig.engine.voices().iter().any(|v| v.full_id() == "beep:low-one"));
    rig.engine
        .say_to(GuildId(G), UserId(ALICE), say("Back to normal."), by.clone())
        .await
        .unwrap();
    assert!(
        rig.tts
            .log()
            .iter()
            .any(|r| r.voice == BeepTts::EN && r.text == "Back to normal.")
    );
    assert_eq!(
        rig.engine.remove_voice("beep:low-one", by).await,
        Err(EngineError::NoSuchVoice)
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_voice_library_is_back_after_a_restart() {
    let rig = Rig::start(Setup::default()).await;
    let by = Actor {
        user: Some(UserId(OWNER)),
        name: None,
        via: Via::Web,
    };
    let staged = rig.engine.upload_dir().join("sample.wav");
    std::fs::write(
        &staged,
        pb_audio::wav16(&tone(300.0, 1.0, LISTEN_RATE, 0.3), LISTEN_RATE),
    )
    .unwrap();
    let rec = rig
        .engine
        .add_voice(
            staged,
            Some("wav".into()),
            "Anna".into(),
            "beep".into(),
            Some("Hi.".into()),
            by,
        )
        .await
        .unwrap();
    let again = Rig::start(Setup {
        dir: Some(rig.stop_keeping_data().await),
        ..Setup::default()
    })
    .await;
    assert_eq!(again.engine.library_voices(), [rec]);
    wait("the voice is given to its model again", 10, || {
        again.engine.voices().iter().any(|v| v.full_id() == "beep:anna")
    })
    .await;
    again.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn muting_or_leaving_ends_the_sentence() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    mic.say(&tone(PROFANE_HZ, 1.0, LISTEN_RATE, 0.3)).await;
    mic.mute(true);
    let muted = rig.wait_sentence("the sentence cut by the mute", |_| true).await;
    assert_eq!(muted.cut, CutCause::Muted);
    assert!(warned(&muted), "{:?}", muted.decision);
    rig.wait_played("its warning", |p| p.sentence == Some(muted.id)).await;

    mic.mute(false);
    echo_fades().await;
    let rest = tone(PROFANE_HZ, 1.0, LISTEN_RATE, 0.3);
    mic.say(&rest).await;
    mic.leave();
    let left = rig
        .wait_sentence("the sentence cut by leaving", |s| s.id != muted.id)
        .await;
    assert_eq!(left.cut, CutCause::Left);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_verdict_is_recorded_but_not_voiced() {
    let gate = Gate::closed();
    let rig = Rig::start(Setup {
        gate: Some(gate.clone()),
        settings: Box::new(|t| set(t, community(), SettingKey::MaxReactionDelay, json!(1.0))),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins().await;
    mic.say(&sentence(PROFANE_HZ)).await;
    wait("the classifier has the sentence", 10, || {
        rig.classified.load(std::sync::atomic::Ordering::SeqCst) >= 1
    })
    .await;
    // The verdict comes after the latest moment a warning may be said.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    gate.open();
    let late = rig
        .wait_sentence("a late verdict", |s| matches!(s.decision, DecisionRecord::Late { .. }))
        .await;
    assert!(
        matches!(
            late.decision,
            DecisionRecord::Late {
                label: Label::Profanity,
                step: 1,
                count: 1,
                ..
            }
        ),
        "{:?}",
        late.decision
    );
    rig.wait_event(
        "its mod log post",
        10,
        |e| matches!(e, Event::MessageSent(m) if m.purpose == MessagePurpose::Modlog { sentence: late.id }),
    )
    .await;

    // A timely sentence after it is the barrier: its warning is said, the late one's never was.
    mic.say(&sentence(PROFANE_HZ)).await;
    let next = rig.wait_sentence("the next sentence", |s| s.id != late.id).await;
    assert!(
        matches!(next.decision, DecisionRecord::Warn { step: 2, count: 2, .. }),
        "{:?}",
        next.decision
    );
    rig.wait_played("its warning", |p| p.sentence == Some(next.id)).await;
    assert!(
        rig.played().await.iter().all(|p| p.sentence != Some(late.id)),
        "the late verdict is not voiced"
    );
    assert_eq!(rig.voice.played_items(GuildId(G), ChannelId(VOICE)).len(), 1);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_warning_is_rendered_before_it_is_needed() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    let warning = "Hey Alice, watch your language!";
    wait("the first warning is rendered ahead of time", 10, || {
        rig.tts.log().iter().any(|r| r.text == warning)
    })
    .await;
    let before = rig.tts.log().len();
    mic.say(&sentence(PROFANE_HZ)).await;
    let p = rig
        .wait_played("the warning", |p| p.purpose == PlayPurpose::Warning)
        .await;
    assert_eq!(p.text.as_deref(), Some(warning));
    assert!(
        rig.tts.log()[before..].iter().all(|r| r.text != warning),
        "rendered again: {:?}",
        rig.tts.log()
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_next_warning_is_rendered_in_the_language_last_heard() {
    let rig = Rig::start(Setup {
        heard: ClfLang::De,
        settings: Box::new(|t| set(t, community(), SettingKey::VoiceLanguage, json!("auto"))),
        ..Setup::default()
    })
    .await;
    let mic = rig.alice_joins().await;
    mic.say(&sentence(PROFANE_HZ)).await;
    let first = rig
        .wait_played("the first warning", |p| p.purpose == PlayPurpose::Warning)
        .await;
    assert_eq!(first.lang.as_ref().map(ToString::to_string).as_deref(), Some("de"));
    // After a decision the person's next warning is prepared: step 2, in the language heard.
    let next = "Alice, das ist schon das zweite Mal. Bitte reiß dich zusammen.";
    wait("the next warning is rendered ahead of time", 10, || {
        rig.tts.log().iter().any(|r| r.voice == BeepTts::DE && r.text == next)
    })
    .await;
    let before = rig.tts.log().len();
    echo_fades().await;
    mic.say(&sentence(PROFANE_HZ)).await;
    let p = rig
        .wait_played("the second warning", |p| {
            p.purpose == PlayPurpose::Warning && p.id != first.id
        })
        .await;
    assert_eq!(p.text.as_deref(), Some(next));
    assert!(
        rig.tts.log()[before..].iter().all(|r| r.text != next),
        "rendered again: {:?}",
        rig.tts.log()
    );
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_disk_does_not_hold_back_the_warning() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    rig.disk.set(Duration::from_millis(2500));
    mic.say(&sentence(PROFANE_HZ)).await;
    let said = tokio::time::Instant::now();
    wait("the warning is played", 10, || {
        !rig.voice.played_items(GuildId(G), ChannelId(VOICE)).is_empty()
    })
    .await;
    let started = rig.voice.played_items(GuildId(G), ChannelId(VOICE))[0].started;
    let waited = started.saturating_duration_since(said);
    assert!(
        waited < Duration::from_millis(1200),
        "the warning waited {waited:?} for the disk"
    );
    // The records still come, in order.
    let s = rig.wait_sentence("the warned sentence", warned).await;
    let p = rig
        .wait_played("the warning's record", |p| p.sentence == Some(s.id))
        .await;
    let events = rig.events().await;
    assert!(
        position(&events, |e| matches!(e, Event::Sentence(x) if x.id == s.id))
            < position(&events, |e| matches!(e, Event::Played(x) if x.id == p.id))
    );
    rig.disk.set(Duration::ZERO);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_records_stopped_last() {
    let rig = Rig::start(Setup::default()).await;
    let _mic = rig.alice_joins().await;
    rig.stop().await;
    let events = rig.events().await;
    assert!(
        matches!(events.first(), Some(Event::Started(_))),
        "{:?}",
        events.first()
    );
    assert_eq!(events.last(), Some(&Event::Stopped(Stopped { clean: true })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_finishes_speech_leaves_voice_then_records_stopped() {
    let rig = Rig::start(Setup::default()).await;
    let mic = rig.alice_joins().await;
    // Alice is mid-sentence when the bot stops.
    let speaking = tokio::spawn(async move { mic.say(&tone(PROFANE_HZ, 3.0, LISTEN_RATE, 0.3)).await });
    tokio::time::sleep(Duration::from_millis(1200)).await;
    rig.stop().await;
    let events = rig.events().await;
    let s = events
        .iter()
        .find_map(|e| match e {
            Event::Sentence(s) => Some(s),
            _ => None,
        })
        .expect("the open sentence was cut and decided");
    assert_eq!(s.cut, CutCause::Shutdown);
    assert!(warned(s), "{:?}", s.decision);
    assert_eq!(
        events.last(),
        Some(&Event::Stopped(Stopped { clean: true })),
        "Stopped comes after everything else"
    );
    assert!(
        rig.fake.bot_connections().is_empty(),
        "the bot left Fluxer's voice channel before the gateway closed"
    );
    assert!(!rig.voice.has_bot(GuildId(G), ChannelId(VOICE)), "the room is closed");
    speaking.abort();
}
