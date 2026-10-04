//! The operator's commands besides `run`: weights, import, the event log and index, the settings files, a health
//! check of the whole installation.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pb_store::{DataLock, FsBlobStore, FsSecretsFile, FsSettingsFiles, JsonlLog, LockError, TursoIndex};
use pb_store_api::{EventLog, SettingsFiles};

use super::Exit;
use super::config::{self, Config};

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().ok()
}

fn load_config(file: &Path) -> Result<Config, Exit> {
    config::load(file).map_err(|e| {
        eprintln!("configuration: {e}");
        Exit::Config
    })
}

/// Takes the data directory's lock (another running bot must not be disturbed).
fn lock(data: &Path) -> Result<DataLock, Exit> {
    DataLock::take(data).map_err(|e| {
        eprintln!("{e}");
        match e {
            LockError::Held(_) => Exit::LockHeld,
            LockError::Io(..) => Exit::Config,
        }
    })
}

/// `pb reset-setup`: the next start asks for a new setup code and a new owner login.
pub fn reset_setup(data: &Path) -> Exit {
    let _lock = match lock(data) {
        Ok(l) => l,
        Err(x) => return x,
    };
    let Some(rt) = runtime() else { return Exit::Internal };
    rt.block_on(async {
        use pb_store_api::{SecretsFile, SessionsFile};
        let secrets = FsSecretsFile::new(data);
        let mut s = match secrets.load().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{e}");
                return Exit::Config;
            }
        };
        s.setup = pb_store_api::SetupState::default();
        if let Err(e) = secrets.save(&s).await {
            eprintln!("{e}");
            return Exit::Internal;
        }
        if let Err(e) = pb_store::FsSessionsFile::new(data).save(&Default::default()).await {
            eprintln!("{e}");
            return Exit::Internal;
        }
        println!("Setup starts again at the next start: the bot's log (and pb setup-code) shows the new code.");
        Exit::Ok
    })
}

/// `pb fetch-weights`: downloads the pinned model weights and voices (or with `check`, only verifies them).
pub fn fetch_weights(file: &Path, dest: Option<&Path>, check: bool) -> Exit {
    let cfg = match load_config(file) {
        Ok(c) => c,
        Err(x) => return x,
    };
    let dest = dest.map_or_else(|| cfg.inference.weights.clone(), Path::to_path_buf);
    let Some(rt) = runtime() else { return Exit::Internal };
    rt.block_on(async {
        if check {
            let mut bad = 0;
            for (w, state) in pb_weights::verify(&dest, true).await {
                let ok = state == pb_weights::State::Ok;
                bad += usize::from(!ok);
                println!("{} {}  {:?}", if ok { "ok  " } else { "BAD " }, w.path, state);
            }
            return if bad == 0 { Exit::Ok } else { Exit::Config };
        }
        if let Err(e) = pb_tls::install_default() {
            eprintln!("{e}");
            return Exit::Config;
        }
        let tls = match pb_tls::client_config() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e}");
                return Exit::Config;
            }
        };
        let Ok(http) = reqwest::Client::builder()
            .tls_backend_preconfigured((*tls).clone())
            .user_agent(concat!("pb-fetch-weights/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .build()
        else {
            return Exit::Internal;
        };
        let mut last = (String::new(), 0u64);
        let result = pb_weights::fetch(&dest, &http, |p| {
            // A line per 5 % per file.
            let pct = p.done * 100 / p.weight.size.max(1);
            if last.0 != p.weight.path || pct >= last.1 + 5 {
                println!("{:>3} %  {}", pct.min(100), p.weight.path);
                last = (p.weight.path.to_owned(), pct);
            }
        })
        .await;
        match result {
            Ok(()) => {
                println!(
                    "all weights and voices are in {} and match their pinned checksums",
                    dest.display()
                );
                Exit::Ok
            }
            Err(e) => {
                eprintln!("{e}");
                Exit::Internal
            }
        }
    })
}

/// `pb import --from <dir>`: imports an old bot's data directory into this (empty) one.
pub fn import(data: &Path, file: &Path, from: &Path) -> Exit {
    let cfg = match load_config(file) {
        Ok(c) => c,
        Err(x) => return x,
    };
    let _lock = match lock(data) {
        Ok(l) => l,
        Err(x) => return x,
    };
    let Some(rt) = runtime() else { return Exit::Internal };
    rt.block_on(async {
        let tmp = data.join("tmp");
        let opened = async {
            let (log, _) = JsonlLog::open(&data.join("log"))?;
            let blobs = FsBlobStore::open(&data.join("blobs"), &tmp).await?;
            Ok::<_, pb_store_api::StoreError>((log, blobs))
        };
        let (log, blobs) = match opened.await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                return Exit::Config;
            }
        };
        let settings = FsSettingsFiles::new(&data.join("settings"));
        let secrets = FsSecretsFile::new(data);
        let (mut tree, problems) = match settings.load().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                return Exit::Config;
            }
        };
        if !problems.is_empty() {
            for p in problems {
                eprintln!("{p}");
            }
            return Exit::Config;
        }
        let targets = pb_import::Targets {
            log: &log,
            blobs: &blobs,
            settings: &settings,
            secrets: &secrets,
            voices_dir: &data.join("voices"),
            scratch: &tmp,
        };
        match super::run::import_old(from, data, &cfg, targets, &mut tree).await {
            Ok(text) => {
                println!("{text}");
                Exit::Ok
            }
            Err(super::run::Fail(code, e)) => {
                eprintln!("{e}");
                code
            }
        }
    })
}

/// `pb store verify`: checks every line of the event log against its hash chain (read only, so it may run beside
/// the bot).
pub fn store_verify(data: &Path) -> Exit {
    match pb_store::verify_dir(&data.join("log")) {
        Ok(r) => {
            println!("{} events in {} segments ({} bytes)", r.events, r.segments, r.bytes);
            for p in &r.problems {
                println!("PROBLEM in {} line {}: {}", p.segment, p.line, p.message);
            }
            if r.ok() { Exit::Ok } else { Exit::Internal }
        }
        Err(e) => {
            eprintln!("{e}");
            Exit::Config
        }
    }
}

/// `pb store rebuild-index`: builds the search index again from the event log (the bot must not be running).
pub fn store_rebuild_index(data: &Path) -> Exit {
    let _lock = match lock(data) {
        Ok(l) => l,
        Err(x) => return x,
    };
    let Some(rt) = runtime() else { return Exit::Internal };
    rt.block_on(async {
        let log: Arc<dyn EventLog> = match JsonlLog::open(&data.join("log")) {
            Ok((log, _)) => Arc::new(log),
            Err(e) => {
                eprintln!("{e}");
                return Exit::Config;
            }
        };
        let dir = data.join("index");
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("{}: {e}", dir.display());
            return Exit::Config;
        }
        let index = match TursoIndex::open(&dir.join("index.db"), log.clone()).await {
            Ok(i) => i,
            Err(e) => {
                eprintln!("{e}");
                return Exit::Internal;
            }
        };
        let head = log.head().map_or(0, |h| h.seq);
        index.caught_up(head).await;
        println!("the index holds all {head} events again");
        Exit::Ok
    })
}

/// `pb settings check`: reads every settings file and reports problems.
pub fn settings_check(data: &Path) -> Exit {
    let Some(rt) = runtime() else { return Exit::Internal };
    rt.block_on(async {
        let files = FsSettingsFiles::new(&data.join("settings"));
        match files.load().await {
            Ok((tree, problems)) => {
                for p in &problems {
                    println!("{p}");
                }
                for (scope, key) in tree.misplaced() {
                    println!("{key:?} is set at {scope:?}, where it does not apply");
                }
                if problems.is_empty() {
                    println!("the settings files are fine");
                    Exit::Ok
                } else {
                    Exit::Config
                }
            }
            Err(e) => {
                eprintln!("{e}");
                Exit::Internal
            }
        }
    })
}

/// `pb settings docs`: every setting as Markdown (name, help, where it can be set, who may change it, default).
pub fn settings_docs(lang: &str) -> Exit {
    use std::io::Write;
    let loc = pb_i18n::Locale::negotiate(lang);
    let mut out = String::from("# Settings\n\n");
    for section in pb_settings::Section::ALL {
        let keys: Vec<_> = pb_settings::SettingKey::all()
            .into_iter()
            .filter(|k| k.meta().section == section)
            .collect();
        if keys.is_empty() {
            continue;
        }
        out.push_str(&format!("## {}\n\n", pb_i18n::section_name(loc, section)));
        for k in keys {
            let m = k.meta();
            let scopes: Vec<String> = m
                .scopes
                .iter()
                .map(|s| {
                    let id = match s {
                        pb_domain::ScopeKind::Global => "scope-global",
                        pb_domain::ScopeKind::Server => "scope-server",
                        pb_domain::ScopeKind::Person => "scope-person",
                    };
                    pb_i18n::text(loc, id, &[])
                })
                .collect();
            let who = match m.who {
                pb_settings::Who::Admins => "who-admins",
                pb_settings::Who::Owner => "who-owner",
            };
            let applies = match m.apply {
                pb_settings::Apply::Live => "apply-live",
                pb_settings::Apply::Reconnect => "apply-reconnect",
            };
            let line = |id: &str, args: &[(&str, pb_i18n::Arg)]| format!("- {}\n", pb_i18n::text(loc, id, args));
            out.push_str(&format!(
                "### {} (`{}`)\n\n{}\n\n",
                pb_i18n::setting_name(loc, k),
                k.name(),
                pb_i18n::setting_help(loc, k)
            ));
            out.push_str(&line("docs-scopes", &[("scopes", scopes.join(", ").into())]));
            out.push_str(&line("docs-who", &[("who", pb_i18n::text(loc, who, &[]).into())]));
            out.push_str(&line(applies, &[]));
            out.push_str(&line("docs-default", &[("value", format!("`{}`", m.default).into())]));
            out.push('\n');
        }
    }
    // A reader that stops early (`| head`) is not an error.
    let _ = std::io::stdout().lock().write_all(out.as_bytes());
    Exit::Ok
}

/// `pb doctor`: checks the installation without starting the bot.
pub fn doctor(data: &Path, file: &Path) -> Exit {
    let mut bad = 0u32;
    let mut say = |ok: bool, what: &str, detail: String| {
        println!(
            "{} {what}{}",
            if ok { "ok  " } else { "FAIL" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
        bad += u32::from(!ok);
    };
    let cfg = match config::load(file) {
        Ok(c) => {
            say(true, "configuration", file.display().to_string());
            c
        }
        Err(e) => {
            say(false, "configuration", e);
            return Exit::Config;
        }
    };
    let missing = pb_tls::missing_cpu_features();
    say(missing.is_empty(), "CPU features for TLS", missing.join(", "));
    let Some(rt) = runtime() else { return Exit::Internal };
    let weights = rt.block_on(pb_weights::verify(&cfg.inference.weights, false));
    let wrong: Vec<String> = weights
        .iter()
        .filter(|(_, s)| *s != pb_weights::State::Ok)
        .map(|(w, s)| format!("{} ({s:?})", w.path))
        .collect();
    say(
        wrong.is_empty(),
        "model weights and voices",
        if wrong.is_empty() {
            cfg.inference.weights.display().to_string()
        } else {
            wrong.join(", ")
        },
    );
    say(
        cfg.inference.espeak_data.is_dir(),
        "espeak-ng data",
        cfg.inference.espeak_data.display().to_string(),
    );
    say(
        cfg.web.site.join("pkg/pb_bg.wasm").is_file(),
        "web UI files",
        cfg.web.site.display().to_string(),
    );
    if let Some(tls) = &cfg.web.tls {
        let r = tls.server_config();
        say(
            r.is_ok(),
            "HTTPS certificate and key",
            r.err().unwrap_or_else(|| tls.cert.display().to_string()),
        );
    }
    say(
        cfg.inference.clips.is_dir(),
        "shipped clips",
        cfg.inference.clips.display().to_string(),
    );
    let writable = std::fs::create_dir_all(data).is_ok() && tempfile_in(data);
    say(writable, "data directory is writable", data.display().to_string());
    let settings = rt.block_on(FsSettingsFiles::new(&data.join("settings")).load());
    match settings {
        Ok((_, p)) => say(
            p.is_empty(),
            "settings files",
            p.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "),
        ),
        Err(e) => say(false, "settings files", e.to_string()),
    }
    if bad == 0 { Exit::Ok } else { Exit::Config }
}

fn tempfile_in(dir: &Path) -> bool {
    let p = dir.join(".pb-doctor");
    let ok = std::fs::write(&p, b"ok").is_ok();
    let _ = std::fs::remove_file(&p);
    ok
}
