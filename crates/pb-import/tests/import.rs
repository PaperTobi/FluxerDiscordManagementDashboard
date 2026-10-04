#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::path::Path;
use std::sync::Arc;

use pb_domain::{GuildId, Scope, UserId};
use pb_import::{ImportError, Source, Targets, import};
use pb_settings::SettingKey;
use pb_store::{FsBlobStore, FsSecretsFile, FsSettingsFiles, JsonlLog, TursoIndex};
use pb_store_api::{AuditFilter, BlobStore, EventLog, Index, SecretsFile, SentenceFilter, SentenceKind, SettingsFiles};
use secrecy::ExposeSecret;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-data")
}

const G: GuildId = GuildId(111_111_111_111_111_111);
const U1: UserId = UserId(222_222_222_222_222_222);
const U2: UserId = UserId(333_333_333_333_333_333);

#[tokio::test(flavor = "multi_thread")]
async fn imports_the_old_data_directory() {
    let data = tempfile::tempdir().unwrap();
    let d = data.path();
    let log: Arc<dyn EventLog> = Arc::new(JsonlLog::open(&d.join("log")).unwrap().0);
    let blobs = FsBlobStore::open(&d.join("blobs"), &d.join("tmp")).await.unwrap();
    let settings = FsSettingsFiles::new(&d.join("settings"));
    let secrets = FsSecretsFile::new(d);
    let (mut tree, _) = settings.load().await.unwrap();
    let src = Source {
        data_dir: fixture(),
        builtin_clips: Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../clips")),
    };
    let targets = Targets {
        log: &*log,
        blobs: &blobs,
        settings: &settings,
        secrets: &secrets,
        voices_dir: &d.join("voices"),
        scratch: &d.join("tmp"),
    };
    let report = import(&src, targets, &mut tree).await.unwrap();
    println!("{}", report.to_text());

    assert_eq!(report.sentences, 8, "7 history rows and one recording without a row");
    assert_eq!(report.from_recordings_only, 1);
    assert_eq!(report.violations, 4);
    assert_eq!(report.recordings, 4);
    assert_eq!(
        report.clips, 3,
        "one upload and the two built-in clips the settings use"
    );
    assert_eq!(
        (
            report.tracked,
            report.people,
            report.audit,
            report.jar,
            report.pending_mutes
        ),
        (3, 2, 4, 1, 1),
        "two tracked in the community, one in every community"
    );
    assert!(report.secrets);
    for removed in [
        "max_per_hour = 20",
        "max_per_hour = 5",
        "max_channels",
        "history_days",
        "evidence_days",
        "audit_days",
        "recent_clips",
        "log_level",
    ] {
        assert!(
            report.notes.iter().any(|n| n.contains(removed)),
            "{removed} is reported: {:?}",
            report.notes
        );
    }

    // The settings files read back to the same tree.
    let (loaded, problems) = settings.load().await.unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(loaded.global, tree.global);
    assert_eq!(loaded.servers, tree.servers);
    let global = loaded.effective(None, None);
    assert!((global.threshold.value.get() - 0.6).abs() < 1e-9);
    assert_eq!(global.voice_language.value.to_string(), "de");
    // Told about every violation (the old default), so every step messages the owner.
    assert!(global.escalation.value.steps().iter().all(|s| s.notify_owner));
    assert_eq!(global.digest_weekday.value.as_str(), "wednesday");
    assert!(global.strike_window.value.value().is_none(), "unlimited");
    assert!(!global.commands_enabled.value);
    assert_eq!(global.join_settle.value.millis(), 2000);
    assert_eq!(global.escalation.value.steps().len(), 2);
    assert_eq!(global.escalation.value.steps()[1].duration.get().millis(), 600_000);
    let community = loaded.effective(Some(G), None);
    assert_eq!(community.strikes.value.get(), 2);
    assert_eq!(community.end_silence.value.get().millis(), 500);
    assert_eq!(community.max_sentence.value.get().millis(), 6000);
    assert_eq!(community.min_voiced.value.get().millis(), 250);
    let person = loaded.effective(Some(G), Some(U1));
    assert!(
        (person.threshold.value.get() - 0.4).abs() < 1e-9,
        "the setting for this community wins over the one for every community"
    );
    assert_eq!(person.strikes.value.get(), 3, "from the settings for every community");
    assert_eq!(person.voice_language.value.to_string(), "en");
    assert!(loaded.effective(Some(G), Some(U2)).observe_only.value);
    assert!(loaded.is_tracked(G, U1) && loaded.is_tracked(G, U2));
    // Tracked in every community.
    let u3 = UserId(444_444_444_444_444_444);
    assert_eq!(global.tracked_everywhere.value, vec![u3]);
    assert!(loaded.is_tracked(G, u3));
    assert!(report.to_text().contains("for every community"), "{}", report.to_text());
    let lines = |s| loaded.voice_lines(s).cloned().unwrap_or_default();
    let warn1 = &lines(Scope::Global)[&"warning.any.1".parse().unwrap()];
    assert_eq!(warn1.text[&"de".parse().unwrap()], "Hey {name}, pass auf!");
    assert_eq!(warn1.clips.len(), 2, "the upload and a built-in clip");
    assert!(lines(Scope::Global).contains_key(&"warning.harassment.1".parse().unwrap()));
    assert_eq!(lines(Scope::Global)[&"greeting".parse().unwrap()].clips.len(), 1);
    assert_eq!(
        lines(Scope::Server { guild: G })[&"greeting".parse().unwrap()].text[&"de".parse().unwrap()],
        "Hallo {name}!"
    );
    let p = lines(Scope::Person { guild: G, user: U1 });
    assert_eq!(
        p[&"name".parse().unwrap()].text.len(),
        2,
        "the spoken name in both built-in languages"
    );
    assert_eq!(
        p[&"warning.any.1".parse().unwrap()].text[&"en".parse().unwrap()],
        "Hey {name}, language!"
    );
    assert!(
        loaded
            .overrides(Scope::Person { guild: G, user: U1 })
            .get_json(SettingKey::TtsVoices)
            .is_some()
    );

    // Secrets.
    let s = secrets.load().await.unwrap();
    assert_eq!(s.bot_token.unwrap().expose_secret(), "1234567890.secretpart");
    assert!(s.setup.done);

    // Everything is in the log and the index sees it.
    let index = TursoIndex::open(&d.join("index/index.db"), log.clone()).await.unwrap();
    index.caught_up(log.head().unwrap().seq).await;
    let all = index.sentences(&SentenceFilter::default(), None, 100).await.unwrap();
    assert_eq!(all.items.len(), 8);
    let violations = index
        .sentences(
            &SentenceFilter {
                kind: SentenceKind::Violations,
                ..SentenceFilter::default()
            },
            None,
            100,
        )
        .await
        .unwrap();
    assert_eq!(violations.items.len(), 4);
    let audio = index
        .sentences(
            &SentenceFilter {
                kind: SentenceKind::WithAudio,
                ..SentenceFilter::default()
            },
            None,
            100,
        )
        .await
        .unwrap();
    assert_eq!(audio.items.len(), 4);
    for row in &audio.items {
        assert!(
            blobs.path(&row.record.audio.unwrap()).await.is_some(),
            "the recording is in the blob store"
        );
    }
    // The decisions by their stored kind.
    let names: Vec<String> = all
        .items
        .iter()
        .map(|r| {
            serde_json::to_value(r.record.decision).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    for want in [
        "nothing_flagged",
        "strike",
        "warn",
        "observe",
        "old_hourly_cap",
        "no_longer_tracked",
    ] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
    let jar = index.jar(Some(G)).await.unwrap();
    assert_eq!(jar.iter().map(|j| (j.user, j.count)).collect::<Vec<_>>(), vec![(U1, 3)]);
    assert_eq!(index.pending_undos().await.unwrap().len(), 1);
    let audit = index
        .audit(
            &AuditFilter {
                kinds: vec!["audit.imported".into()],
                ..AuditFilter::default()
            },
            None,
            100,
        )
        .await
        .unwrap();
    assert_eq!(audit.items.len(), 4);
    assert_eq!(index.people(&[U1], None).await.unwrap()[0].shown(), "User One");
    assert_eq!(index.clips().await.unwrap().len(), 3);
    assert!(log.verify().await.unwrap().ok());

    // Only once.
    let again = import(&src, targets, &mut tree).await;
    assert!(matches!(again, Err(ImportError::LogNotEmpty)));
}
