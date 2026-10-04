//! The client against the fake Fluxer (the documented wire format).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use pb_domain::{ChannelId, GuildId, MessageId, UserId};
use pb_fluxer::{FluxerClient, GatewayConfig};
use pb_fluxer_api::{
    Attachment, Destination, ErrorKind, Fluxer, FluxerCtl, GatewayEvent, LoginError, MemberPatch, MessageRef,
    OAuthClient, OutgoingMessage, VoiceGrant, VoiceStateOp,
};
use pb_fluxer_fake::{FakeConfig, FakeFluxer};
use secrecy::SecretString;
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

const G: u64 = 111;
const VOICE: u64 = 222;
const TEXT: u64 = 333;
const ALICE: u64 = 444;

fn client() -> FluxerClient {
    FluxerClient {
        gateway: GatewayConfig {
            backoff_base: Duration::from_millis(30),
            backoff_max: Duration::from_millis(200),
            rate_limit_backoff: Duration::from_millis(100),
            op3_per_window: 2,
            op3_window: Duration::from_millis(300),
            ..GatewayConfig::default()
        },
    }
}

async fn fake(cfg: FakeConfig) -> FakeFluxer {
    let f = FakeFluxer::start(cfg).await;
    f.add_user(ALICE, "alice", Some("Alice"));
    f.add_guild(G, "Alpha", ALICE, 1 << 10);
    f.add_channel(G, VOICE, "voice", 2);
    f.add_channel(G, TEXT, "chat", 0);
    f.add_member(G, ALICE, &[]);
    f
}

async fn login(f: &FakeFluxer) -> (Arc<dyn FluxerCtl>, UnboundedReceiver<GatewayEvent>) {
    let c = client();
    let ep = c.discover(&Url::parse(&f.url()).unwrap()).await.unwrap();
    c.login(&ep, &SecretString::from(f.config().token.clone()))
        .await
        .unwrap()
}

async fn next(rx: &mut UnboundedReceiver<GatewayEvent>, what: &str, f: impl Fn(&GatewayEvent) -> bool) -> GatewayEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let ev = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no {what}"))
            .expect("events end");
        if f(&ev) {
            return ev;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_in_and_reads_communities_and_messages() {
    let f = fake(FakeConfig::default()).await;
    let (ctl, mut rx) = login(&f).await;
    assert_eq!(ctl.me().user.id, UserId(1001));
    assert_eq!(ctl.me().owner, Some(UserId(1002)));
    let GatewayEvent::Ready { guilds, .. } = next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await
    else {
        panic!()
    };
    assert_eq!(guilds, vec![GuildId(G)]);
    let GatewayEvent::GuildAvailable(g) = next(&mut rx, "GUILD_CREATE", |e| {
        matches!(e, GatewayEvent::GuildAvailable(_))
    })
    .await
    else {
        panic!()
    };
    assert_eq!(
        (g.name.as_str(), g.owner, g.channels.len()),
        ("Alpha", Some(UserId(ALICE)), 2)
    );
    assert_eq!(g.roles[0].permissions, 1 << 10);
    // Fluxer names the members by id only here; their users come from READY.
    let bot = g
        .members
        .iter()
        .find(|m| m.id == UserId(1001))
        .expect("the bot is a member");
    assert!(bot.user.as_ref().is_some_and(|u| !u.username.is_empty()), "{bot:?}");
    f.say(G, TEXT, ALICE, &[7], "!pb status");
    let GatewayEvent::Message(m) = next(&mut rx, "MESSAGE_CREATE", |e| matches!(e, GatewayEvent::Message(_))).await
    else {
        panic!()
    };
    assert_eq!(
        (m.content.as_str(), m.author.id, m.guild),
        ("!pb status", UserId(ALICE), Some(GuildId(G)))
    );
    assert_eq!(m.author_roles, vec![pb_domain::RoleId(7)]);
    f.voice_join(G, VOICE, ALICE);
    let GatewayEvent::VoiceState { state: vs, .. } =
        next(&mut rx, "voice state", |e| matches!(e, GatewayEvent::VoiceState { .. })).await
    else {
        panic!()
    };
    assert_eq!((vs.user, vs.channel), (UserId(ALICE), Some(ChannelId(VOICE))));
    ctl.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_token_is_rejected_before_the_gateway() {
    let f = fake(FakeConfig::default()).await;
    let c = client();
    let ep = c.discover(&Url::parse(&f.url()).unwrap()).await.unwrap();
    let r = c.login(&ep, &SecretString::from("1000.wrong".to_owned())).await;
    assert!(matches!(r, Err(LoginError::TokenRejected)));
    let bad = c.discover(&Url::parse("http://127.0.0.1:9/").unwrap()).await;
    assert!(matches!(bad, Err(LoginError::Unreachable(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn joins_confirms_and_leaves_voice() {
    let f = fake(FakeConfig::default()).await;
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    ctl.voice_state(VoiceStateOp::Join {
        guild: GuildId(G),
        channel: ChannelId(VOICE),
    })
    .await
    .unwrap();
    let GatewayEvent::VoiceServer(grant) = next(&mut rx, "grant", |e| matches!(e, GatewayEvent::VoiceServer(_))).await
    else {
        panic!()
    };
    let VoiceGrant::LiveKit {
        connection, endpoint, ..
    } = *grant
    else {
        panic!()
    };
    assert_eq!(endpoint.scheme(), "wss");
    assert!(f.bot_connections()[0].3, "pending until confirmed");
    ctl.voice_state(VoiceStateOp::Update {
        guild: GuildId(G),
        channel: ChannelId(VOICE),
        connection: connection.clone(),
    })
    .await
    .unwrap();
    f.wait_until(Duration::from_secs(2), "confirmed", |f| {
        f.bot_connections().first().is_some_and(|c| !c.3)
    })
    .await;
    ctl.voice_state(VoiceStateOp::Leave {
        guild: GuildId(G),
        connection,
    })
    .await
    .unwrap();
    f.wait_until(Duration::from_secs(2), "left", |f| f.bot_connections().is_empty())
        .await;
    let updates = f.voice_updates();
    assert!(
        updates[0].get("connection_id").is_none(),
        "a join carries no connection id"
    );
    assert!(updates[2]["channel_id"].is_null(), "leaving sends a null channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unconfirmed_join_lapses() {
    let f = fake(FakeConfig {
        pending_timeout: Duration::from_millis(300),
        ..FakeConfig::default()
    })
    .await;
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    ctl.voice_state(VoiceStateOp::Join {
        guild: GuildId(G),
        channel: ChannelId(VOICE),
    })
    .await
    .unwrap();
    f.wait_until(Duration::from_secs(2), "pending", |f| f.bot_connections().len() == 1)
        .await;
    f.wait_until(Duration::from_secs(3), "dropped", |f| f.bot_connections().is_empty())
        .await;
    // A pending join is never announced: no voice state came for it.
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, GatewayEvent::VoiceState { state, .. } if state.user == UserId(1001)),
            "a pending join was announced"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_and_replays_what_it_missed() {
    let f = fake(FakeConfig::default()).await;
    let (_ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    next(&mut rx, "GUILD_CREATE", |e| {
        matches!(e, GatewayEvent::GuildAvailable(_))
    })
    .await;
    f.drop_connections(4000);
    // Said while the bot is away: replayed on resume.
    f.say(G, TEXT, ALICE, &[], "while you were gone");
    let GatewayEvent::Down { resuming, code } = next(&mut rx, "down", |e| matches!(e, GatewayEvent::Down { .. })).await
    else {
        panic!()
    };
    assert!(resuming);
    assert_eq!(code, Some(4000));
    let mut saw_message = false;
    loop {
        match next(&mut rx, "resumed", |_| true).await {
            GatewayEvent::Message(m) => saw_message |= m.content == "while you were gone",
            GatewayEvent::Resumed => break,
            GatewayEvent::Ready { .. } => panic!("identified instead of resuming"),
            _ => {}
        }
    }
    assert!(saw_message, "the missed message was replayed before RESUMED");
    // op 7: reconnect and resume again.
    f.ask_reconnect();
    next(&mut rx, "resumed after op 7", |e| matches!(e, GatewayEvent::Resumed)).await;
    // A session the server forgot: op 9, then a fresh login on the same socket.
    f.forget_sessions();
    next(&mut rx, "a new READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_replies_files_splitting_and_rate_limits() {
    let f = fake(FakeConfig::default()).await;
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    let said = f.say(G, TEXT, ALICE, &[], "!pb help");
    f.rate_limit_next(&format!("POST /channels/{TEXT}/messages"), 0.2, false);
    let started = std::time::Instant::now();
    let m = OutgoingMessage {
        content: "hello <@444>".into(),
        reply_to: Some(MessageId(said)),
        ping: vec![UserId(ALICE)],
        files: vec![],
    };
    ctl.send(Destination::Channel(ChannelId(TEXT)), m).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(190), "waited out the 429");
    let sent = f.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].payload["message_reference"]["message_id"], said.to_string());
    assert_eq!(sent[0].payload["allowed_mentions"]["users"][0], ALICE.to_string());
    assert_eq!(sent[0].payload["allowed_mentions"]["replied_user"], false);
    // A reply to a message that is gone goes out plainly.
    let gone = OutgoingMessage {
        content: "late".into(),
        reply_to: Some(MessageId(1)),
        ..OutgoingMessage::default()
    };
    ctl.send(Destination::Channel(ChannelId(TEXT)), gone).await.unwrap();
    assert!(f.sent()[1].payload.get("message_reference").is_none());
    // Files go as multipart; long text is split.
    let wav = Attachment {
        filename: "s.wav".into(),
        media_type: "audio/wav".into(),
        bytes: Bytes::from_static(b"RIFF...."),
    };
    let long = OutgoingMessage {
        content: format!("{}\n{}", "a".repeat(3000), "b".repeat(3000)),
        files: vec![wav],
        ..OutgoingMessage::default()
    };
    ctl.send(Destination::Channel(ChannelId(TEXT)), long).await.unwrap();
    let sent = f.sent();
    assert_eq!(sent.len(), 4);
    assert!(sent[2].files.is_empty());
    assert_eq!(sent[3].files, vec![("s.wav".to_owned(), b"RIFF....".to_vec())]);
    // A message whose answer was lost is sent again with its nonce, and Fluxer creates it once.
    f.lose_next_answer();
    ctl.send(
        Destination::Channel(ChannelId(TEXT)),
        OutgoingMessage {
            content: "once".into(),
            ..OutgoingMessage::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(f.sent().iter().filter(|m| m.content() == "once").count(), 1);
    // White space alone is not sent.
    let blank = ctl
        .send(
            Destination::Channel(ChannelId(TEXT)),
            OutgoingMessage {
                content: " \n ".into(),
                ..OutgoingMessage::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(blank.kind, ErrorKind::BadRequest);
    assert_eq!(f.sent().len(), 5);
    // A global rate limit pauses everything.
    f.rate_limit_next("POST /channels", 0.15, true);
    ctl.send(
        Destination::Channel(ChannelId(TEXT)),
        OutgoingMessage {
            content: "x".into(),
            ..OutgoingMessage::default()
        },
    )
    .await
    .unwrap();
    // Reactions and direct messages.
    ctl.react(
        MessageRef {
            channel: ChannelId(TEXT),
            message: MessageId(said),
        },
        "✅",
    )
    .await
    .unwrap();
    assert_eq!(f.reactions()[0].2, "✅");
    ctl.send(
        Destination::User(UserId(ALICE)),
        OutgoingMessage {
            content: "dm".into(),
            ..OutgoingMessage::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(f.sent().last().unwrap().channel, f.dm_channel(ALICE).unwrap());
    f.refuse_dms(ALICE);
    let refused = ctl
        .send(
            Destination::User(UserId(ALICE)),
            OutgoingMessage {
                content: "dm".into(),
                ..OutgoingMessage::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Forbidden);
    assert!(refused.is_code("CANNOT_SEND_MESSAGES_TO_USER"));
}

#[tokio::test(flavor = "multi_thread")]
async fn members_moderation_presence_and_oauth() {
    let f = fake(FakeConfig::default()).await;
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    let m = ctl.member(GuildId(G), UserId(ALICE)).await.unwrap().unwrap();
    assert_eq!(m.shown(), "Alice");
    assert!(ctl.member(GuildId(G), UserId(9)).await.unwrap().is_none());
    let not_in_voice = ctl
        .patch_member(
            GuildId(G),
            UserId(ALICE),
            MemberPatch {
                disconnect: true,
                ..MemberPatch::default()
            },
        )
        .await
        .unwrap_err();
    assert!(not_in_voice.is_code("USER_NOT_IN_VOICE"));
    f.voice_join(G, VOICE, ALICE);
    ctl.patch_member(
        GuildId(G),
        UserId(ALICE),
        MemberPatch {
            mute: Some(true),
            reason: Some("Stufe 2 · step 2".into()),
            ..MemberPatch::default()
        },
    )
    .await
    .unwrap();
    let until: jiff::Timestamp = "2030-01-01T00:00:00Z".parse().unwrap();
    ctl.patch_member(
        GuildId(G),
        UserId(ALICE),
        MemberPatch {
            timeout_until: Some(Some(until)),
            reason: Some("Stufe 3 · step 3".into()),
            ..MemberPatch::default()
        },
    )
    .await
    .unwrap();
    let patches = f.patches();
    assert_eq!(patches[0].body["mute"], true);
    assert_eq!(
        patches[0].reason.as_deref(),
        Some("Stufe 2  step 2"),
        "the header keeps the ASCII part"
    );
    assert_eq!(patches[1].body["communication_disabled_until"], "2030-01-01T00:00:00Z");
    assert_eq!(
        patches[1].reason.as_deref(),
        Some("Stufe 3 · step 3"),
        "a time-out keeps it in full"
    );
    // Presence: the latest value wins, paced.
    for i in 0..5 {
        ctl.presence(Some(format!("watching {i}")));
    }
    f.wait_until(Duration::from_secs(3), "the last presence", |f| {
        f.presences()
            .last()
            .is_some_and(|p| p["custom_status"]["text"] == "watching 4")
    })
    .await;
    assert!(f.presences().len() <= 3, "{:?}", f.presences());
    // OAuth2.
    let c = client();
    let ep = c.discover(&Url::parse(&f.url()).unwrap()).await.unwrap();
    let oauth = OAuthClient {
        client_id: 1000,
        client_secret: SecretString::from("client-secret".to_owned()),
        redirect_uri: Url::parse("http://bot.local/auth/callback").unwrap(),
    };
    let verifier = "v".repeat(43);
    let code = f.oauth_code(ALICE, oauth.redirect_uri.as_str(), &verifier);
    let user = c.oauth_user(&ep, &oauth, &code, &verifier).await.unwrap();
    assert_eq!((user.id, user.global_name.as_deref()), (UserId(ALICE), Some("Alice")));
    let again = c.oauth_user(&ep, &oauth, &code, &verifier).await.unwrap_err();
    assert_eq!(again.kind, ErrorKind::Unauthorized, "a code works once");
    let other = f.oauth_code(ALICE, oauth.redirect_uri.as_str(), &verifier);
    let wrong = c.oauth_user(&ep, &oauth, &other, &"w".repeat(43)).await.unwrap_err();
    assert_eq!(
        wrong.kind,
        ErrorKind::Unauthorized,
        "the verifier must match the challenge"
    );
    let bad = c.oauth_user(&ep, &oauth, "nope", &verifier).await.unwrap_err();
    assert_eq!(bad.kind, ErrorKind::Unauthorized);
    let url = pb_fluxer_api::authorize_url(&ep, &oauth, "st", "ch");
    assert!(url.contains("code_challenge_method=S256") && url.contains("scope=identify"));
}

#[tokio::test(flavor = "multi_thread")]
async fn searches_members_by_name_through_the_gateway() {
    let f = fake(FakeConfig::default()).await;
    f.add_user(5001, "alice", Some("Alice"));
    f.add_user(5002, "albert", None);
    f.add_user(5003, "bob", None);
    for u in [5001, 5002, 5003] {
        f.add_member(G, u, &[]);
    }
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    let mut found: Vec<u64> = ctl
        .search_members(GuildId(G), "al", 10)
        .await
        .unwrap()
        .iter()
        .map(|m| m.id.0)
        .collect();
    found.sort_unstable();
    assert_eq!(found, vec![ALICE, 5001, 5002], "everyone whose name starts with \"al\"");
    let one = ctl.search_members(GuildId(G), "al", 1).await.unwrap();
    assert_eq!(one.len(), 1, "the limit is passed on");
    assert!(
        one[0].user.as_ref().is_some_and(|u| !u.username.is_empty()),
        "with names"
    );
    // The members also reach the directory as member updates.
    next(&mut rx, "a member update", |e| {
        matches!(e, GatewayEvent::MemberUpdated { .. })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn searches_while_typing_go_one_at_a_time_and_the_newest_wins() {
    // Fluxer works on one search at a time and replaces a waiting one with a newer one: sent all at once, the middle
    // search would never be answered.
    let f = fake(FakeConfig {
        search_delay: Duration::from_millis(300),
        ..FakeConfig::default()
    })
    .await;
    f.add_user(5001, "alice", Some("Alice"));
    f.add_member(G, 5001, &[]);
    let (ctl, mut rx) = login(&f).await;
    next(&mut rx, "READY", |e| matches!(e, GatewayEvent::Ready { .. })).await;
    let started = std::time::Instant::now();
    let search = |q: &'static str| {
        let ctl = ctl.clone();
        tokio::spawn(async move { ctl.search_members(GuildId(G), q, 10).await })
    };
    let a = search("a");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let al = search("al");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let ali = search("ali");
    let first = a.await.unwrap().unwrap();
    assert!(first.iter().any(|m| m.id.0 == 5001));
    let middle = al.await.unwrap().unwrap_err();
    assert_eq!(middle.kind, ErrorKind::Superseded, "the newer search replaced it");
    let last = ali.await.unwrap().unwrap();
    assert_eq!(last.iter().map(|m| m.id.0).collect::<Vec<_>>(), vec![ALICE, 5001]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "nobody waited for a timeout"
    );
}
