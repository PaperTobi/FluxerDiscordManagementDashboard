//! Contract tests every implementation must pass. Call them from the implementation's tests.

#![allow(clippy::unwrap_used, clippy::missing_panics_doc)]

use std::future::Future;

use bytes::Bytes;
use futures::StreamExt;
use jiff::Timestamp;
use pb_domain::{BlobHash, GuildId, UserId};

use super::{Actor, BlobStore, Event, EventLog, GENESIS, JarReset, NewEvent, StoredEvent, Via, WriterHealth};

fn ev(i: u32) -> NewEvent {
    NewEvent {
        kind: "test".into(),
        v: 1,
        ts: None,
        data: serde_json::json!({ "i": i }),
    }
}

async fn all<L: EventLog + ?Sized>(log: &L, from: u64) -> Vec<StoredEvent> {
    log.scan(from).map(|r| r.unwrap()).collect().await
}

/// The event log. `open` must open the same storage every time it is called (after the previous instance is
/// dropped).
pub async fn event_log<L, F, Fut>(open: F)
where
    L: EventLog,
    F: Fn() -> Fut,
    Fut: Future<Output = L>,
{
    {
        let log = open().await;
        assert_eq!(log.head(), None);
        assert!(all(&log, 1).await.is_empty());
        assert!(log.verify().await.unwrap().ok());
        assert_eq!(log.health(), WriterHealth::Ok);

        let mut follow = log.follow();
        let given: Timestamp = "2024-02-03T04:05:06.789Z".parse().unwrap();
        let mut first = ev(1);
        first.ts = Some(given);
        let refs = log.append(vec![first, ev(2)]).await.unwrap();
        assert_eq!(refs.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2]);
        let batch = follow.recv().await.unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].ts, given, "a given time is kept");
        assert_eq!(batch[0].prev, GENESIS);
        assert_eq!(batch[1].prev, batch[0].hash);
        let refs = log.append(vec![ev(3)]).await.unwrap();
        assert_eq!(refs[0].seq, 3);
        assert_eq!(log.head(), Some(refs[0]));
        assert!(log.append(Vec::new()).await.unwrap().is_empty());

        // Concurrent appends get unique, contiguous numbers.
        let mut tasks = Vec::new();
        for i in 0..20 {
            tasks.push(log.append(vec![ev(100 + i), ev(200 + i)]));
        }
        let mut seqs: Vec<u64> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .flat_map(|r| r.unwrap())
            .map(|r| r.seq)
            .collect();
        seqs.sort_unstable();
        assert_eq!(seqs, (4..44).collect::<Vec<_>>());

        let events = all(&log, 1).await;
        assert_eq!(events.len(), 43);
        for w in events.windows(2) {
            assert_eq!(w[1].seq, w[0].seq + 1);
            assert_eq!(w[1].prev, w[0].hash);
        }
        assert_eq!(all(&log, 40).await.first().map(|e| e.seq), Some(40));
        let report = log.verify().await.unwrap();
        assert!(report.ok(), "{report:?}");
        assert_eq!(report.events, 43);
        assert!(log.size() > 0);
    }
    // Reopened: everything is there and numbering continues.
    let log = open().await;
    assert_eq!(log.head().map(|h| h.seq), Some(43));
    let refs = log.append(vec![ev(9)]).await.unwrap();
    assert_eq!(refs[0].seq, 44);
    let events = all(&log, 43).await;
    assert_eq!(events[1].prev, events[0].hash);

    // Typed events survive the trip.
    let typed = Event::JarReset(JarReset {
        guild: GuildId(1),
        user: UserId(2),
        by: Actor {
            user: Some(UserId(3)),
            name: None,
            via: Via::Web,
        },
    });
    log.append(vec![typed.to_new(None).unwrap()]).await.unwrap();
    let back = all(&log, 45).await;
    assert_eq!(Event::from_stored(&back[0]).unwrap(), typed);
}

/// The blob store (on an empty store).
pub async fn blob_store<B: BlobStore>(store: &B) {
    let a = store.put(Bytes::from_static(b"hello")).await.unwrap();
    assert!(a.new);
    assert_eq!(a.size, 5);
    assert_eq!(
        a.hash.hex(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    let again = store.put(Bytes::from_static(b"hello")).await.unwrap();
    assert!(!again.new, "the same content is stored once");
    assert_eq!(store.get(&a.hash).await.unwrap().as_deref(), Some(&b"hello"[..]));
    assert!(store.path(&a.hash).await.is_some_and(|p| p.exists()));

    let staged = store.staging_dir().join("upload.bin");
    std::fs::write(&staged, b"staged bytes").unwrap();
    let b = store.put_file(&staged).await.unwrap();
    assert!(!staged.exists(), "the staged file was moved in");
    assert_eq!(store.get(&b.hash).await.unwrap().as_deref(), Some(&b"staged bytes"[..]));
    assert!(store.size() >= 17);

    assert!(store.delete(&a.hash).await.unwrap());
    assert!(!store.delete(&a.hash).await.unwrap());
    assert_eq!(store.get(&a.hash).await.unwrap(), None);
    assert_eq!(store.path(&a.hash).await, None);
    let missing: BlobHash = "sha256:0000000000000000000000000000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    assert_eq!(store.get(&missing).await.unwrap(), None);
}

// ------------------------------------------------------------------------------------------------ index

use std::sync::Arc;

use pb_domain::{
    ActionKind, ActionOutcome, Audience, ChannelId, ClfLang, Label, MessageId, PlayPurpose, Scope, SentenceId,
};

use super::{
    ActionRecord, AuditFilter, BlobDeleted, ChatDeleted, ChatRecord, ClipRecord, ClipRemoved, CommunitySeen, CutCause,
    DecisionRecord, Index, MessagePurpose, MessageSent, Page, PersonSeen, PlayOutcome, PlayRecord, SentenceFilter,
    SentenceKind, SentenceRecord, SentenceSource, SettingsChanged, VoiceRecord, VoiceRemoved,
};

fn sentence(user: u64, at: &str, decision: DecisionRecord, jar: bool, audio: Option<BlobHash>) -> SentenceRecord {
    let flagged = if matches!(decision, DecisionRecord::NothingFlagged) {
        vec![]
    } else {
        vec![Label::Profanity]
    };
    SentenceRecord {
        id: SentenceId::new(),
        guild: GuildId(1),
        channel: ChannelId(10),
        user: UserId(user),
        started: at.parse().unwrap(),
        dur_ms: 1500,
        level_db: Some(-20.0),
        cut: CutCause::Pause,
        scores: [0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.9, 0.0],
        language: ClfLang::De,
        thresholds: vec![(Label::Profanity, 0.5)],
        flagged,
        decision,
        jar,
        audio,
        infer_ms: Some(120),
        cut_to_verdict_ms: Some(200),
        model: "test".into(),
        source: SentenceSource::Live,
    }
}

async fn wait_for<I: Index>(index: &I, seq: u64) {
    for _ in 0..500 {
        if index.applied() >= seq {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the index did not reach event {seq} (at {})", index.applied());
}

fn at(s: &str) -> Option<Timestamp> {
    Some(s.parse().unwrap())
}

fn hash(n: u8) -> BlobHash {
    BlobHash::from_bytes([n; 32])
}

/// The query index. `open` gives a fresh, empty log and an index following it.
pub async fn index<I, F, Fut>(open: F)
where
    I: Index,
    F: Fn() -> Fut,
    Fut: Future<Output = (Arc<dyn EventLog>, I)>,
{
    let (log, index) = open().await;
    let me = Actor {
        user: Some(UserId(99)),
        name: Some("owner".into()),
        via: Via::Web,
    };
    let warn = |label, step, count| DecisionRecord::Warn {
        label,
        score: 0.9,
        step,
        count,
    };
    let clean = sentence(5, "2026-10-01T10:00:00Z", DecisionRecord::NothingFlagged, false, None);
    let strike = sentence(
        5,
        "2026-10-01T10:01:00Z",
        DecisionRecord::Strike { strike: 1, of: 2 },
        false,
        None,
    );
    let warned = sentence(
        5,
        "2026-10-01T22:30:00Z",
        warn(Label::Profanity, 1, 1),
        true,
        Some(hash(1)),
    );
    let observed = sentence(
        5,
        "2026-10-02T09:00:00Z",
        DecisionRecord::Observe {
            label: Label::Harassment,
            score: 0.8,
            step: 2,
            count: 2,
        },
        true,
        Some(hash(2)),
    );
    // Someone in another community.
    let other = SentenceRecord {
        guild: GuildId(2),
        ..sentence(6, "2026-10-02T09:05:00Z", warn(Label::Profanity, 1, 1), false, None)
    };
    let mute = ActionRecord {
        id: SentenceId::new(),
        guild: GuildId(1),
        user: UserId(5),
        kind: ActionKind::Mute,
        secs: Some(300),
        sentence: Some(observed.id),
        step: Some(2),
        outcome: ActionOutcome::Done,
        undo_at: at("2026-10-02T09:05:00Z"),
        undoes: None,
        retry_at: None,
    };
    let play = PlayRecord {
        id: SentenceId::new(),
        guild: GuildId(1),
        channel: ChannelId(10),
        person: Some(UserId(5)),
        purpose: PlayPurpose::Warning,
        sentence: Some(warned.id),
        line: Some("warning.profanity.1".into()),
        clips: vec![],
        text: Some("Hey".into()),
        lang: Some("de".parse().unwrap()),
        audience: Audience::Tracked,
        started: "2026-10-01T22:30:02Z".parse().unwrap(),
        dur_ms: 900,
        outcome: PlayOutcome::Played,
        by: None,
    };
    let clip = ClipRecord {
        render: hash(7),
        original: hash(8),
        name: "beep".into(),
        lang: None,
        transcript: None,
        dur_ms: 500,
        self_check: None,
        heard_language: None,
        added_by: me.clone(),
        by: me.clone(),
    };
    let change = pb_settings::Change::Set {
        scope: Scope::Server { guild: GuildId(1) },
        key: "strikes".into(),
        before: None,
        after: serde_json::json!(2),
    };
    let events = vec![
        (
            Event::CommunitySeen(CommunitySeen {
                guild: GuildId(1),
                name: "Alpha".into(),
                icon: None,
            }),
            None,
        ),
        (
            Event::PersonSeen(PersonSeen {
                user: UserId(5),
                guild: None,
                username: "five".into(),
                display_name: Some("Five".into()),
                nick: None,
                avatar: None,
            }),
            None,
        ),
        (
            Event::PersonSeen(PersonSeen {
                user: UserId(5),
                guild: Some(GuildId(1)),
                username: "five".into(),
                display_name: Some("Five".into()),
                nick: Some("V".into()),
                avatar: None,
            }),
            None,
        ),
        (Event::Sentence(Box::new(clean.clone())), at("2026-10-01T10:00:02Z")),
        (Event::Sentence(Box::new(strike.clone())), at("2026-10-01T10:01:02Z")),
        (Event::Sentence(Box::new(warned.clone())), at("2026-10-01T22:30:02Z")),
        (Event::Played(Box::new(play)), at("2026-10-01T22:30:03Z")),
        (Event::Sentence(Box::new(observed.clone())), at("2026-10-02T09:00:02Z")),
        (Event::Action(Box::new(mute.clone())), at("2026-10-02T09:00:03Z")),
        (Event::Sentence(Box::new(other.clone())), at("2026-10-02T09:05:02Z")),
        (
            Event::SettingsChanged(Box::new(SettingsChanged { change, by: me.clone() })),
            at("2026-10-02T10:00:00Z"),
        ),
        (Event::ClipSaved(Box::new(clip.clone())), at("2026-10-02T10:01:00Z")),
        (
            Event::BlobDeleted(BlobDeleted {
                hash: hash(1),
                by: me.clone(),
                reason: None,
            }),
            at("2026-10-02T10:02:00Z"),
        ),
        (
            Event::MessageSent(MessageSent {
                purpose: MessagePurpose::Digest {
                    from: "2026-10-01T00:00:00Z".parse().unwrap(),
                    until: "2026-10-03T00:00:00Z".parse().unwrap(),
                },
                guild: None,
                channel: None,
                ok: true,
                error: None,
                with_audio: false,
            }),
            at("2026-10-03T00:00:05Z"),
        ),
    ];
    let new: Vec<NewEvent> = events.iter().map(|(e, ts)| e.to_new(*ts).unwrap()).collect();
    let refs = log.append(new).await.unwrap();
    wait_for(&index, refs.last().unwrap().seq).await;

    // Sentences, newest first, in pages.
    let all = SentenceFilter {
        user: Some(UserId(5)),
        ..SentenceFilter::default()
    };
    let p1 = index.sentences(&all, None, 3).await.unwrap();
    assert_eq!(
        p1.items.iter().map(|r| r.record.id).collect::<Vec<_>>(),
        vec![observed.id, warned.id, strike.id]
    );
    let p2 = index.sentences(&all, p1.next, 3).await.unwrap();
    assert_eq!(p2.items.iter().map(|r| r.record.id).collect::<Vec<_>>(), vec![clean.id]);
    assert_eq!(p2.next, None);
    let kind = |k| SentenceFilter {
        guilds: Some(vec![GuildId(1)]),
        kind: k,
        ..SentenceFilter::default()
    };
    assert_eq!(
        index
            .sentences(&kind(SentenceKind::Flagged), None, 50)
            .await
            .unwrap()
            .items
            .len(),
        3,
        "the other community's sentence is not counted"
    );
    assert_eq!(
        index
            .sentences(&kind(SentenceKind::Violations), None, 50)
            .await
            .unwrap()
            .items
            .len(),
        2
    );
    let with_audio = index.sentences(&kind(SentenceKind::WithAudio), None, 50).await.unwrap();
    assert_eq!(
        with_audio.items.iter().map(|r| r.record.id).collect::<Vec<_>>(),
        vec![observed.id],
        "a deleted recording is not listed"
    );
    let within = |gs: Vec<GuildId>| SentenceFilter {
        guilds: Some(gs),
        ..SentenceFilter::default()
    };
    let count = async |gs: Vec<GuildId>| index.sentences(&within(gs), None, 50).await.unwrap().items.len();
    assert_eq!(count(vec![GuildId(2)]).await, 1);
    assert_eq!(
        count(vec![GuildId(2), GuildId(1)]).await,
        count(vec![GuildId(1)]).await + 1,
        "several communities"
    );
    assert!(
        index
            .sentences(&within(Vec::new()), None, 50)
            .await
            .unwrap()
            .items
            .is_empty(),
        "no community: nothing"
    );
    let harassment = SentenceFilter {
        label: Some(Label::Harassment),
        kind: SentenceKind::Violations,
        ..SentenceFilter::default()
    };
    assert_eq!(index.sentences(&harassment, None, 50).await.unwrap().items.len(), 1);
    let since = SentenceFilter {
        since: at("2026-10-02T00:00:00Z"),
        ..SentenceFilter::default()
    };
    assert_eq!(index.sentences(&since, None, 50).await.unwrap().items.len(), 2);
    let row = index.sentence(warned.id).await.unwrap().unwrap();
    assert!(row.audio_deleted);
    assert_eq!(row.record, warned);
    assert_eq!(index.sentence(SentenceId::new()).await.unwrap(), None);

    // Days in the reporting time zone: 22:30 UTC on Oct 1 is Oct 2 in Berlin.
    let days = index
        .days(
            GuildId(1),
            UserId(5),
            "2026-10-01".parse().unwrap(),
            "2026-10-03".parse().unwrap(),
            "Europe/Berlin",
        )
        .await
        .unwrap();
    assert_eq!(
        days.iter()
            .map(|d| (d.sentences, d.flagged, d.violations))
            .collect::<Vec<_>>(),
        vec![(2, 1, 0), (2, 2, 2), (0, 0, 0)]
    );
    assert_eq!(days[0].speech_ms, 3000);

    // The swear jar counts violations marked for it.
    let jar = index.jar(Some(GuildId(1))).await.unwrap();
    assert_eq!(
        jar.iter().map(|j| (j.user, j.count)).collect::<Vec<_>>(),
        vec![(UserId(5), 2)]
    );

    let times = index.violation_times().await.unwrap();
    assert_eq!(
        times.iter().map(|t| t.1).collect::<Vec<_>>(),
        vec![UserId(5), UserId(5), UserId(6)],
        "every violation, oldest first"
    );

    // A timed mute is pending until its undo succeeds; a failed undo moves its due time.
    let pending = index.pending_undos().await.unwrap();
    assert_eq!(pending.iter().map(|a| a.id).collect::<Vec<_>>(), vec![mute.id]);
    let undo = |outcome, retry_at| ActionRecord {
        id: SentenceId::new(),
        kind: ActionKind::Unmute,
        secs: None,
        sentence: None,
        step: None,
        outcome,
        undo_at: None,
        undoes: Some(mute.id),
        retry_at,
        ..mute.clone()
    };
    let failed = undo(
        ActionOutcome::Failed {
            error: "HTTP 500".into(),
        },
        at("2026-10-02T09:10:00Z"),
    );
    let r = log
        .append(vec![Event::Action(Box::new(failed)).to_new(None).unwrap()])
        .await
        .unwrap();
    wait_for(&index, r[0].seq).await;
    assert_eq!(
        index.pending_undos().await.unwrap()[0].undo_at,
        at("2026-10-02T09:10:00Z")
    );
    let r = log
        .append(vec![
            Event::Action(Box::new(undo(ActionOutcome::Done, None)))
                .to_new(None)
                .unwrap(),
        ])
        .await
        .unwrap();
    wait_for(&index, r[0].seq).await;
    assert!(index.pending_undos().await.unwrap().is_empty());

    let audit = index.audit(&AuditFilter::default(), None, 100).await.unwrap();
    let kinds: Vec<&str> = audit.items.iter().map(|a| a.event.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            "action",
            "action",
            "message.sent",
            "blob.deleted",
            "clip.saved",
            "settings.changed",
            "action"
        ]
    );
    let community = index
        .audit(
            &AuditFilter {
                guilds: Some(vec![GuildId(1)]),
                kinds: vec!["settings.changed".into()],
                ..AuditFilter::default()
            },
            None,
            100,
        )
        .await
        .unwrap();
    assert_eq!(community.items.len(), 1);

    assert_eq!(
        index
            .clips()
            .await
            .unwrap()
            .iter()
            .map(|c| c.record.name.as_str())
            .collect::<Vec<_>>(),
        vec!["beep"]
    );
    let r = log
        .append(vec![
            Event::ClipRemoved(ClipRemoved {
                render: hash(7),
                by: me.clone(),
            })
            .to_new(None)
            .unwrap(),
        ])
        .await
        .unwrap();
    wait_for(&index, r[0].seq).await;
    assert!(index.clips().await.unwrap().is_empty());

    // Flagged chat messages: listed newest first, a deletion noted, and their violations counted with the sentences'.
    let before = index.violation_times().await.unwrap().len();
    let chat = |n: u8, violation: bool| ChatRecord {
        id: SentenceId::new(),
        guild: GuildId(1),
        channel: ChannelId(4),
        message: MessageId(u64::from(n)),
        user: UserId(5),
        at: at("2026-10-02T12:00:00Z").unwrap(),
        text: format!("message {n}"),
        matches: vec!["fuck*".into()],
        decision: if violation {
            DecisionRecord::Warn {
                label: Label::Profanity,
                score: 1.0,
                step: 1,
                count: 1,
            }
        } else {
            DecisionRecord::Strike { strike: 1, of: 2 }
        },
        jar: violation,
    };
    let (strike, warned) = (chat(1, false), chat(2, true));
    let r = log
        .append(vec![
            Event::ChatFlagged(Box::new(strike)).to_new(None).unwrap(),
            Event::ChatFlagged(Box::new(warned.clone())).to_new(None).unwrap(),
            Event::ChatDeleted(ChatDeleted {
                id: warned.id,
                ok: true,
                error: None,
            })
            .to_new(None)
            .unwrap(),
        ])
        .await
        .unwrap();
    wait_for(&index, r[2].seq).await;
    let rows = index.chat(Some(GuildId(1)), None, 10).await.unwrap().items;
    assert_eq!(
        rows.iter().map(|r| r.record.text.as_str()).collect::<Vec<_>>(),
        ["message 2", "message 1"]
    );
    assert_eq!((rows[0].deleted, rows[1].deleted), (Some(true), None));
    assert_eq!(index.violation_times().await.unwrap().len(), before + 1);
    assert!(index.chat(Some(GuildId(2)), None, 10).await.unwrap().items.is_empty());

    // The voice library: added, renamed (keeping when it was added), removed.
    let voice = |id: &str, name: &str| VoiceRecord {
        model: "omnivoice".into(),
        id: id.into(),
        name: name.into(),
        sample: hash(9),
        data: hash(10),
        transcript: Some("Hello there.".into()),
        added_by: me.clone(),
        by: me.clone(),
    };
    let r = log
        .append(
            [
                (voice("anna", "Anna"), "2026-10-02T11:00:00Z"),
                (voice("bob", "Bob"), "2026-10-02T11:01:00Z"),
                (voice("anna", "Anna (calm)"), "2026-10-02T11:02:00Z"),
            ]
            .into_iter()
            .map(|(v, t)| Event::VoiceSaved(Box::new(v)).to_new(at(t)).unwrap())
            .chain(std::iter::once(
                Event::VoiceRemoved(VoiceRemoved {
                    model: "omnivoice".into(),
                    id: "bob".into(),
                    by: me.clone(),
                })
                .to_new(None)
                .unwrap(),
            ))
            .collect(),
        )
        .await
        .unwrap();
    wait_for(&index, r[3].seq).await;
    let voices = index.voices().await.unwrap();
    assert_eq!(voices.len(), 1);
    assert_eq!(voices[0].record.voice_id(), "omnivoice:anna");
    assert_eq!(voices[0].record.name, "Anna (calm)");
    assert_eq!(Some(voices[0].added), at("2026-10-02T11:00:00Z"));

    let people = index.people(&[UserId(5), UserId(404)], Some(GuildId(1))).await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].shown(), "V");
    assert_eq!(index.people(&[UserId(5)], None).await.unwrap()[0].shown(), "Five");
    assert_eq!(index.communities().await.unwrap()[0].name, "Alpha");

    let last = index.last_digest().await.unwrap();
    assert_eq!(
        (last.tried, last.sent, last.error),
        (at("2026-10-03T00:00:00Z"), at("2026-10-03T00:00:00Z"), None)
    );
    let digest = index
        .digest(
            "2026-10-01T00:00:00Z".parse().unwrap(),
            "2026-10-03T00:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        digest
            .iter()
            .map(|d| (d.user, d.violations, d.max_step, d.actions))
            .collect::<Vec<_>>(),
        vec![(UserId(5), 2, 2, 1), (UserId(6), 1, 1, 0)]
    );

    let r = log
        .append(vec![
            Event::JarReset(JarReset {
                guild: GuildId(1),
                user: UserId(5),
                by: me,
            })
            .to_new(None)
            .unwrap(),
        ])
        .await
        .unwrap();
    wait_for(&index, r[0].seq).await;
    assert!(index.jar(None).await.unwrap().is_empty());

    let count = async |kind: SentenceKind| {
        let f = SentenceFilter {
            kind,
            ..SentenceFilter::default()
        };
        index.sentences(&f, None, 100).await.unwrap().items.len()
    };
    assert_eq!(
        (
            count(SentenceKind::All).await,
            count(SentenceKind::Violations).await,
            count(SentenceKind::WithAudio).await
        ),
        (5, 3, 1)
    );
    assert_eq!(index.applied(), log.head().unwrap().seq);
    let _: Page<()> = Page::default();
}
