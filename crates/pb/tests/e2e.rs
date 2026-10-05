//! The whole bot against a fake Fluxer and a real LiveKit server, with the real models.
//!
//! Needs `LIVEKIT_SERVER` (a livekit-server binary) and `PB_WEIGHTS` (the models and voices); run with `--ignored`.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use std::time::Duration;

use common::*;
use pb_domain::PlayPurpose;
use pb_domain::{ActionOutcome, GuildId, Scope};
use pb_settings::SettingKey;
use pb_store_api::{DecisionRecord, Event, Index, PlayOutcome};

fn is_warned(e: &Event) -> bool {
    matches!(e, Event::Sentence(s) if matches!(s.decision, DecisionRecord::Warn { .. }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn follows_hears_warns_records_and_reports() {
    let rig = Rig::start(Setup::default()).await;
    let (alice, mic) = rig.alice_joins().await;
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    rig.wait_event("a warned sentence", 30, is_warned).await;
    wait("Alice hears the warning", 20, || rig.alice_heard(&alice) > 1600).await;
    wait("the mod log post", 10, || {
        rig.fake
            .sent()
            .iter()
            .any(|m| m.channel == TEXT && m.content().contains(&format!("<@{ALICE}>")))
    })
    .await;
    let Event::Played(p) = rig
        .wait_event("the playback record", 20, |e| matches!(e, Event::Played(_)))
        .await
    else {
        panic!()
    };
    assert!(p.ok(), "{p:?}");
    assert_eq!(p.lang.as_ref().map(ToString::to_string).as_deref(), Some("en"));
    assert_eq!(p.purpose, PlayPurpose::Warning);
    // The index sees it too.
    rig.index.caught_up(rig.log.head().unwrap().seq).await;
    let rows = pb_store_api::Index::sentences(
        &*rig.index,
        &pb_store_api::SentenceFilter {
            kind: pb_store_api::SentenceKind::WithAudio,
            ..Default::default()
        },
        None,
        10,
    )
    .await
    .unwrap();
    assert_eq!(rows.items.len(), 1, "the flagged sentence's recording was kept");
    // The owner gets a direct message (without the recording: the default).
    wait("the owner's direct message", 10, || {
        rig.fake
            .dm_channel(OWNER)
            .is_some_and(|c| rig.fake.sent().iter().any(|m| m.channel == c && m.files.is_empty()))
    })
    .await;
    // Alice leaves voice; the bot leaves too, after the leave delay (5 s).
    rig.fake.voice_leave(&rig.alice_connection());
    wait("the bot leaves", 15, || rig.fake.bot_connections().is_empty()).await;
    alice.leave().await.unwrap();
    rig.stop(None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn a_benign_sentence_is_scored_but_not_warned() {
    let rig = Rig::start(Setup::default()).await;
    let (alice, mic) = rig.alice_joins().await;
    mic.say(&speech48("benign_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    let Event::Sentence(s) = rig
        .wait_event("a scored sentence", 30, |e| matches!(e, Event::Sentence(_)))
        .await
    else {
        panic!()
    };
    assert_eq!(s.decision, DecisionRecord::NothingFlagged);
    assert!(s.audio.is_none(), "an unflagged sentence keeps no recording");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!rig.events().await.iter().any(|e| matches!(e, Event::Played(_))));
    rig.stop(Some(alice)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn chat_commands_switch_to_german_and_report_status() {
    let rig = Rig::start(Setup::default()).await;
    let (alice, mic) = rig.alice_joins().await;
    let cmd = rig
        .fake
        .say(G, TEXT, OWNER, &[], &format!("!pb set language de <@{ALICE}>"));
    wait("the command's answer", 10, || {
        rig.fake
            .sent()
            .iter()
            .any(|m| m.payload["message_reference"]["message_id"].as_str() == Some(cmd.to_string().as_str()))
    })
    .await;
    assert!(
        rig.fake.reactions().iter().any(|(_, m, e)| *m == cmd && e == "✅"),
        "{:?}",
        rig.fake.reactions()
    );
    let status = rig.fake.say(G, TEXT, ALICE, &[], "!pb status");
    wait("the status", 10, || {
        rig.fake.sent().iter().any(|m| {
            m.payload["message_reference"]["message_id"].as_str() == Some(status.to_string().as_str())
                && m.content().contains("voice")
        })
    })
    .await;
    // Alice may not change settings.
    let denied = rig.fake.say(G, TEXT, ALICE, &[], "!pb pause");
    wait("the refusal", 10, || {
        rig.fake.reactions().iter().any(|(_, m, e)| *m == denied && e == "🚫")
    })
    .await;
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    let Event::Played(p) = rig
        .wait_event("the playback record", 40, |e| matches!(e, Event::Played(_)))
        .await
    else {
        panic!()
    };
    assert_eq!(p.lang.as_ref().map(ToString::to_string).as_deref(), Some("de"), "{p:?}");
    assert!(p.ok());
    rig.stop(Some(alice)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn strikes_then_an_escalation_mute_that_is_lifted() {
    let rig = Rig::start(Setup {
        settings: Box::new(|t| {
            let g = Scope::Server { guild: GuildId(G) };
            let mut c = Vec::new();
            c.extend(t.set(g, SettingKey::Strikes, serde_json::json!(2), true).unwrap());
            c.extend(
                t.set(g, SettingKey::StrikeNotice, serde_json::json!(true), true)
                    .unwrap(),
            );
            c.extend(
                t.set(g, SettingKey::ActionsEnabled, serde_json::json!(true), true)
                    .unwrap(),
            );
            c.extend(
                t.set(
                    g,
                    SettingKey::Escalation,
                    serde_json::json!([{"from": 1, "action": "mute", "duration": 2}]),
                    true,
                )
                .unwrap(),
            );
            c
        }),
        ..Setup::default()
    })
    .await;
    let (alice, mic) = rig.alice_joins().await;
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    rig.wait_event(
        "a strike",
        30,
        |e| matches!(e, Event::Sentence(s) if matches!(s.decision, DecisionRecord::Strike { strike: 1, of: 2 })),
    )
    .await;
    // Speech over the bot's own voice is not scored (its echo could be in it), so the next sentence waits.
    rig.wait_event(
        "the strike notice",
        30,
        |e| matches!(e, Event::Played(p) if p.purpose == PlayPurpose::StrikeNotice),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    rig.wait_event("the warning", 30, is_warned).await;
    let Event::Action(a) = rig
        .wait_event("the mute", 20, |e| matches!(e, Event::Action(a) if a.undoes.is_none()))
        .await
    else {
        panic!()
    };
    assert_eq!(a.outcome, ActionOutcome::Done);
    wait("Fluxer was asked to mute", 5, || {
        rig.fake
            .patches()
            .iter()
            .any(|p| p.user == ALICE && p.body["mute"] == true)
    })
    .await;
    rig.wait_event(
        "the mute is lifted",
        15,
        |e| matches!(e, Event::Action(a) if a.undoes.is_some() && a.outcome == ActionOutcome::Done),
    )
    .await;
    assert!(
        rig.fake
            .patches()
            .iter()
            .any(|p| p.user == ALICE && p.body["mute"] == false)
    );
    rig.stop(Some(alice)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn without_speak_it_writes_in_the_channel_chat_and_greets() {
    let rig = Rig::start(Setup {
        can_speak: false,
        settings: Box::new(|t| {
            t.set(
                Scope::Server { guild: GuildId(G) },
                SettingKey::GreetEnabled,
                serde_json::json!(true),
                true,
            )
            .unwrap()
            .into_iter()
            .collect()
        }),
        ..Setup::default()
    })
    .await;
    let (alice, mic) = rig.alice_joins().await;
    // The greeting goes into the voice channel's chat too.
    wait("the greeting in chat", 20, || {
        rig.fake
            .sent()
            .iter()
            .any(|m| m.channel == VOICE && m.content().contains(&format!("<@{ALICE}>")))
    })
    .await;
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    rig.wait_event(
        "the warning written in chat",
        40,
        |e| matches!(e, Event::Played(p) if p.outcome == PlayOutcome::Written && p.purpose == PlayPurpose::Warning && p.sentence.is_some()),
    )
    .await;
    rig.stop(Some(alice)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn a_gateway_resume_keeps_the_bot_in_the_call() {
    let rig = Rig::start(Setup::default()).await;
    let (alice, mic) = rig.alice_joins().await;
    let before = rig.fake.bot_connections();
    rig.fake.drop_connections(4000);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        rig.fake.bot_connections(),
        before,
        "the same connection after the resume"
    );
    mic.say(&speech48("profane_1.wav")).await.unwrap();
    mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
    rig.wait_event("a warned sentence", 30, is_warned).await;
    wait("Alice hears the warning", 20, || rig.alice_heard(&alice) > 1600).await;
    rig.stop(Some(alice)).await;
}
