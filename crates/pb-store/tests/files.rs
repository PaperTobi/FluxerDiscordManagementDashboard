#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_domain::{GuildId, Scope, UserId};
use pb_settings::{SettingKey, SettingsTree};
use pb_store::{FsSecretsFile, FsSessionsFile, FsSettingsFiles};
use pb_store_api::{Secrets, SecretsFile, SessionRecord, SessionsFile, SettingsFiles, SetupState};
use secrecy::{ExposeSecret, SecretString};

#[tokio::test]
async fn settings_are_written_where_they_belong_and_hand_comments_stay() {
    let dir = tempfile::tempdir().unwrap();
    let files = FsSettingsFiles::new(&dir.path().join("settings"));
    let (mut tree, problems) = files.load().await.unwrap();
    assert!(problems.is_empty());
    assert_eq!(tree, SettingsTree::default());

    let g = GuildId(42);
    let u = UserId(7);
    let mut changes = Vec::new();
    changes.extend(
        tree.set(Scope::Global, SettingKey::Threshold, serde_json::json!(0.6), true)
            .unwrap(),
    );
    changes.extend(
        tree.set(
            Scope::Server { guild: g },
            SettingKey::Strikes,
            serde_json::json!(2),
            false,
        )
        .unwrap(),
    );
    changes.extend(tree.track(g, u, Some(UserId(1)), "2026-10-04T10:00:00Z".parse().unwrap()));
    changes.extend(
        tree.set(
            Scope::Person { guild: g, user: u },
            SettingKey::Threshold,
            serde_json::json!(0.3),
            false,
        )
        .unwrap(),
    );
    files.write(&changes).await.unwrap();

    let server_file = dir.path().join("settings/servers/42.toml");
    let text = std::fs::read_to_string(&server_file).unwrap();
    std::fs::write(&server_file, format!("# my note about this community\n{text}")).unwrap();

    let (loaded, problems) = files.load().await.unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(loaded.global, tree.global);
    assert_eq!(loaded.servers, tree.servers);
    assert!(loaded.is_tracked(g, u));
    assert!((loaded.effective(Some(g), Some(u)).threshold.value.get() - 0.3).abs() < 1e-9);

    let mut tree = loaded;
    let change = tree
        .clear(Scope::Server { guild: g }, SettingKey::Strikes, false)
        .unwrap()
        .unwrap();
    files.write(&[change]).await.unwrap();
    let text = std::fs::read_to_string(&server_file).unwrap();
    assert!(text.starts_with("# my note about this community\n"), "{text}");
    assert!(!text.contains("strikes"), "{text}");
}

#[tokio::test]
async fn broken_files_are_reported_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("settings/servers")).unwrap();
    std::fs::write(
        dir.path().join("settings/global.toml"),
        "schema = 1\n[settings]\nthreshold = 7\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("settings/servers/oops.toml"), "").unwrap();
    let (tree, problems) = FsSettingsFiles::new(&dir.path().join("settings")).load().await.unwrap();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(tree, SettingsTree::default());
    assert!(problems[0].span.is_some(), "the bad value is located");
}

#[tokio::test]
async fn secrets_and_sessions_round_trip_privately() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let secrets = FsSecretsFile::new(dir.path());
    assert!(secrets.load().await.unwrap().bot_token.is_none());
    let s = Secrets {
        bot_token: Some(SecretString::from("123.abc")),
        client_secret: None,
        cookie_key: Some(SecretString::from("00ff")),
        setup: SetupState {
            done: true,
            owner: Some(UserId(5)),
            finished_at: None,
        },
    };
    secrets.save(&s).await.unwrap();
    let back = secrets.load().await.unwrap();
    assert_eq!(
        back.bot_token.as_ref().map(|t| t.expose_secret().to_owned()).as_deref(),
        Some("123.abc")
    );
    assert_eq!(back.setup, s.setup);
    let mode = std::fs::metadata(dir.path().join("secrets.toml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    secrets.write_setup_code(Some("ABCD-EFGH")).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("setup-code")).unwrap(),
        "ABCD-EFGH\n"
    );
    secrets.write_setup_code(None).await.unwrap();
    assert!(!dir.path().join("setup-code").exists());

    let sessions = FsSessionsFile::new(dir.path());
    let t: jiff::Timestamp = "2026-10-04T10:00:00Z".parse().unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert(
        "h1".to_owned(),
        SessionRecord {
            user: UserId(5),
            name: "o".into(),
            avatar: None,
            owner: true,
            created: t,
            expires: t,
            fresh_until: t,
            last_seen: t,
        },
    );
    sessions.save(&map).await.unwrap();
    assert_eq!(sessions.load().await.unwrap(), map);
}
