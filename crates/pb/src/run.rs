//! `pb run`: opens the data directory, loads the models once, starts the engine and the web server, and stops cleanly
//! on SIGTERM or Ctrl-C. SIGHUP reads the settings and voice files again.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use pb_engine::{Deps, Engine, ShippedClip, SystemClock};
use pb_infer::{Inference, Models};
use pb_models_api::TtsEngine;
use pb_settings::SettingsTree;
use pb_store::{
    DataLock, FsBlobStore, FsSecretsFile, FsSessionsFile, FsSettingsFiles, JsonlLog, LockError, TursoIndex,
};
use pb_store_api::{BlobStore, EventLog, SettingsFiles};
use tokio::sync::watch;

use super::Exit;
use super::config::{self, Config, Device};

/// A failed start: what to tell the operator and how to exit.
pub(crate) struct Fail(pub(crate) Exit, pub(crate) String);

impl<E: std::fmt::Display> From<(Exit, E)> for Fail {
    fn from((x, e): (Exit, E)) -> Self {
        Fail(x, e.to_string())
    }
}

pub fn main(data: &Path, config_file: &Path) -> Exit {
    let cfg = match config::load(config_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pb: configuration: {e}");
            return Exit::Config;
        }
    };
    let _log_guard = logging(data, &cfg);
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "could not start the async runtime");
            return Exit::Internal;
        }
    };
    match rt.block_on(run(data, cfg)) {
        Ok(()) => Exit::Ok,
        Err(Fail(code, msg)) => {
            tracing::error!("{msg}");
            eprintln!("pb: {msg}");
            code
        }
    }
}

/// Logs to stderr and to daily files (`<data>/logs/pb.log.YYYY-MM-DD`, kept).
fn logging(data: &Path, cfg: &Config) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| cfg.logging.level.clone());
    let dir = cfg.logging.dir.clone().unwrap_or_else(|| data.join("logs"));
    let (file, guard) = match tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("pb.log")
        .build(&dir)
    {
        Ok(appender) => {
            let (w, g) = tracing_appender::non_blocking(appender);
            (
                Some(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(w)),
                Some(g),
            )
        }
        Err(e) => {
            eprintln!("pb: no log files in {}: {e}", dir.display());
            (None, None)
        }
    };
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_new(&filter).unwrap_or_else(|_| "info".into()))
        // Colours only on a terminal: the journal and `podman logs` keep escape codes as text.
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
                .with_writer(std::io::stderr),
        )
        .with(file)
        .try_init();
    guard
}

async fn run(data: &Path, cfg: Config) -> Result<(), Fail> {
    tracing::info!(version = env!("CARGO_PKG_VERSION"), data = %data.display(), "starting");
    // TLS needs certain CPU instructions; say so now rather than at the first connection.
    pb_tls::install_default().map_err(|e| Fail(Exit::Config, e.to_string()))?;
    let _lock = DataLock::take(data).map_err(|e| match e {
        LockError::Held(_) => Fail(Exit::LockHeld, e.to_string()),
        LockError::Io(..) => Fail(Exit::Config, e.to_string()),
    })?;
    // The web UI's port, before anything starts (a busy port is a configuration problem with nothing to undo).
    let listener = tokio::net::TcpListener::bind(cfg.web.bind).await.map_err(|e| {
        let hint = if e.kind() == std::io::ErrorKind::AddrInUse {
            " (another program uses this port, maybe another copy of the bot: stop it, or choose another address with \
             PB__WEB__BIND=0.0.0.0:8791 or `bind` under [web] in config.toml)"
        } else {
            ""
        };
        Fail(Exit::Config, format!("web UI address {}: {e}{hint}", cfg.web.bind))
    })?;

    // The store.
    let tmp = data.join("tmp");
    let (log, repaired) = JsonlLog::open(&data.join("log")).map_err(|e| Fail(Exit::Config, e.to_string()))?;
    let log: Arc<dyn EventLog> = Arc::new(log);
    if let Some(r) = repaired {
        tracing::warn!(?r, "the event log had a torn last line; it was cut off");
        // Recorded, so the audit trail shows it.
        if let Some(e) = pb_store_api::Event::LogRepaired(r).to_new(None) {
            log.append(vec![e])
                .await
                .map_err(|e| Fail(Exit::Internal, e.to_string()))?;
        }
    }
    let index = Arc::new(
        TursoIndex::open(&data.join("index/index.db"), log.clone())
            .await
            .map_err(|e| Fail(Exit::Internal, e.to_string()))?,
    );
    let blobs: Arc<dyn BlobStore> = Arc::new(
        FsBlobStore::open(&data.join("blobs"), &tmp)
            .await
            .map_err(|e| Fail(Exit::Config, e.to_string()))?,
    );
    let settings_files = Arc::new(FsSettingsFiles::new(&data.join("settings")));
    let secrets =
        Arc::new(super::secrets::EnvSecrets::new(FsSecretsFile::new(data)).map_err(|e| Fail(Exit::Config, e))?);
    let (token_from_env, client_secret_from_env) = secrets.given_by_env();
    let sessions = Arc::new(FsSessionsFile::new(data));
    let (mut tree, problems) = settings_files
        .load()
        .await
        .map_err(|e| Fail(Exit::Internal, e.to_string()))?;
    if !problems.is_empty() {
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        return Err(Fail(
            Exit::Config,
            format!("settings files have errors:\n{}", text.join("\n")),
        ));
    }

    // The old bot's data, once.
    if pb_import::pending(data, &*log) {
        tracing::info!("found the old bot's data (bot.db): importing it once");
        let targets = pb_import::Targets {
            log: &*log,
            blobs: &*blobs,
            settings: &*settings_files,
            secrets: &*secrets,
            voices_dir: &data.join("voices"),
            scratch: &tmp,
        };
        let text = import_old(data, data, &cfg, targets, &mut tree).await?;
        tracing::info!("import finished (report in import-report.txt):\n{text}");
    }

    // The models, loaded once for the life of the process.
    let inference = models(data, &cfg, &tree)?;
    let shipped_clips = shipped_clips(&cfg.inference.clips, &*blobs).await;

    let disk_dir = data.to_path_buf();
    let deps = Deps {
        fluxer: Arc::new(pb_fluxer::FluxerClient::default()),
        voice: Arc::new(pb_voice_livekit::LiveKitTransport),
        inference: inference.clone(),
        log: log.clone(),
        index: index.clone(),
        blobs: blobs.clone(),
        settings_files,
        secrets: secrets.clone(),
        hub: pb_live::Hub::new(),
        clock: Arc::new(SystemClock::default()),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        shipped_clips,
        disk_free: Arc::new(move || disk_free(&disk_dir)),
    };
    let engine = Arc::new(
        Engine::start(deps, {
            // `config.toml`'s [defaults], below the global settings (in place before anything reads them).
            tree.file_defaults = cfg.defaults.clone();
            tree
        })
        .await
        .map_err(|e| Fail(Exit::Internal, e.to_string()))?,
    );

    // The web server.
    let (stop_tx, stop_rx) = watch::channel(false);
    let web_cfg = pb_web_server::WebConfig {
        bind: cfg.web.bind,
        site_root: cfg.web.site.clone(),
        live: pb_live::SessionCfg::default(),
        setup_code_file: data.join("setup-code").display().to_string(),
        token_from_env,
        client_secret_from_env,
        tls: match &cfg.web.tls {
            Some(t) => Some(
                t.server_config()
                    .map_err(|e| Fail(Exit::Config, format!("HTTPS: {e}")))?,
            ),
            None => None,
        },
    };
    let api = pb_api::Api::load(
        engine.clone(),
        index.clone(),
        Arc::new(pb_store::FsApiFile::new(data)),
    )
    .await
    .map_err(|e| Fail(Exit::Config, format!("api.json: {e}")))?;
    let parts = pb_web_server::WebParts {
        engine: engine.clone(),
        index: index.clone(),
        blobs: blobs.clone(),
        log: log.clone(),
        secrets,
        sessions,
        api,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        shutdown: stop_rx,
    };
    let state = pb_web_server::WebState::new(web_cfg, parts)
        .await
        .map_err(|e| Fail(Exit::Internal, e.to_string()))?;
    tracing::info!(addr = %cfg.web.bind, https = cfg.web.tls.is_some(), "web UI listening");
    let web = tokio::spawn(pb_web_server::serve(state, listener));

    let fatal = tokio::select! {
        () = signals(&engine) => None,
        f = engine.fatal() => Some(f),
    };
    match &fatal {
        Some(f) => tracing::error!(error = %f, "stopping: a part of the bot failed for good (it is started again)"),
        None => tracing::info!("stopping"),
    }
    let _ = stop_tx.send(true);
    // Live pages close at once; a request in progress gets 2 s.
    match tokio::time::timeout(Duration::from_secs(2), web).await {
        Ok(Ok(Err(e))) => tracing::warn!(error = %e, "the web server stopped with an error"),
        Err(_) => tracing::warn!("the web server did not stop within 2 s"),
        _ => {}
    }
    engine.shutdown().await;
    // The model threads are joined off the async threads.
    let models = tokio::task::spawn_blocking(move || inference.shutdown());
    if tokio::time::timeout(Duration::from_secs(5), models).await.is_err() {
        tracing::warn!("the model threads did not stop within 5 s");
    }
    tracing::info!("stopped");
    match fatal {
        Some(f) => Err(Fail(Exit::Internal, f.to_string())),
        None => Ok(()),
    }
}

/// Waits for SIGTERM or Ctrl-C; SIGHUP reads the settings and voice files again on the way.
async fn signals(engine: &Engine) {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    loop {
        tokio::select! {
            _ = term.recv() => return,
            _ = tokio::signal::ctrl_c() => return,
            _ = hup.recv() => match engine.reload().await {
                Ok(problems) if problems.is_empty() => tracing::info!("settings files read again"),
                Ok(problems) => {
                    for p in problems {
                        tracing::error!("{p} (the old values stay in use)");
                    }
                }
                Err(e) => tracing::error!(error = %e, "settings files could not be read again"),
            },
        }
    }
}

/// Imports the old (Python) bot's data directory `from` into this one (`data`, opened as `targets`); the report is
/// returned and written to `import-report.txt`.
pub(crate) async fn import_old(
    from: &Path,
    data: &Path,
    cfg: &Config,
    targets: pb_import::Targets<'_>,
    tree: &mut SettingsTree,
) -> Result<String, Fail> {
    let src = pb_import::Source {
        data_dir: from.to_path_buf(),
        builtin_clips: Some(cfg.inference.clips.clone()).filter(|p| p.is_dir()),
    };
    let report = pb_import::import(&src, targets, tree)
        .await
        .map_err(|e| Fail(Exit::Internal, format!("importing the old data failed: {e}")))?;
    let text = report.to_text();
    if let Err(e) = tokio::fs::write(data.join("import-report.txt"), &text).await {
        tracing::warn!(error = %e, "the import report could not be written");
    }
    Ok(text)
}

fn models(data: &Path, cfg: &Config, tree: &SettingsTree) -> Result<Inference, Fail> {
    let w = &cfg.inference.weights;
    let hint = "run `pb fetch-weights` or set PB__INFERENCE__WEIGHTS";
    let vad = pb_vad_silero::SileroVad::load(&w.join("silero-vad")).map_err(|e| {
        Fail(
            Exit::Config,
            format!("voice activity model in {}: {e} ({hint})", w.display()),
        )
    })?;
    let eff = tree.effective(None, None);
    let threads = NonZeroUsize::new(eff.cpu_threads.value.get() as usize).unwrap_or(NonZeroUsize::MIN);
    let dir = w.join("roblox-voice-safety-v3");
    let cpu = || pb_classifier_roblox::RobloxClassifier::load_cpu(&dir, threads).map(|c| Box::new(c) as _);
    #[cfg(feature = "gpu")]
    let gpu = || {
        pb_classifier_roblox::RobloxClassifier::load_gpu(&dir, pb_classifier_roblox::GpuDevice::DiscreteGpu(0))
            .map(|c| Box::new(c) as _)
    };
    #[cfg(not(feature = "gpu"))]
    let gpu = || {
        Err(pb_models_api::ModelError::Load(
            "this build has no GPU support (the `gpu` feature)".to_owned(),
        ))
    };
    let classifier: Box<dyn pb_models_api::Classifier> = match cfg.inference.device {
        Device::Cpu => cpu(),
        Device::Gpu => gpu(),
        Device::Auto => gpu().or_else(|e| {
            tracing::info!(reason = %e, "the classifier runs on the CPU (no usable GPU)");
            cpu()
        }),
    }
    .map_err(|e| Fail(Exit::Config, format!("classifier in {}: {e} ({hint})", dir.display())))?;
    tracing::info!(device = %classifier.info().device, "classifier loaded");
    let espeak = cfg.inference.espeak_data.clone();
    if !espeak.is_dir() {
        return Err(Fail(
            Exit::Config,
            format!(
                "espeak-ng data not found in {} (set PB__INFERENCE__ESPEAK_DATA)",
                espeak.display()
            ),
        ));
    }
    let voice_dirs: Vec<PathBuf> = vec![w.join("voices"), data.join("voices")];
    Inference::start(Models {
        vad: Box::new(vad),
        classifier,
        tts: vec![Box::new(move |threads| {
            pb_tts_piper::PiperEngine::new(&espeak, &voice_dirs, threads).map(|e| Box::new(e) as Box<dyn TtsEngine>)
        })],
        tts_threads: eff.tts_threads.value.get() as usize,
    })
    .map_err(|e| Fail(Exit::Internal, format!("the model threads could not start: {e}")))
}

/// The shipped clips, prepared like uploads (48 kHz, −16 LUFS) and stored as blobs. A clip that cannot be read is
/// skipped with a warning.
async fn shipped_clips(dir: &Path, blobs: &dyn BlobStore) -> Vec<ShippedClip> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "no shipped clips in {}", dir.display());
            return Vec::new();
        }
    };
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let prepared = std::fs::read(&f)
            .map_err(|e| e.to_string())
            .and_then(|b| pb_audio::decode(&b, Some("wav")).map_err(|e| e.to_string()))
            .and_then(|pcm| pb_audio::prepare_clip(&pcm).map_err(|e| e.to_string()));
        let samples = match prepared {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(file = %f.display(), error = %e, "a shipped clip could not be read");
                continue;
            }
        };
        match blobs
            .put(Bytes::from(pb_audio::wav16(&samples, pb_audio::PLAY_RATE)))
            .await
        {
            Ok(info) => out.push(ShippedClip {
                hash: info.hash,
                text: clip_text(&f),
            }),
            Err(e) => tracing::warn!(file = %f.display(), error = %e, "a shipped clip could not be stored"),
        }
    }
    out
}

/// `watch_your_mouth.wav` → "Watch your mouth".
fn clip_text(f: &Path) -> String {
    let stem = f
        .file_stem()
        .map(|s| s.to_string_lossy().replace('_', " "))
        .unwrap_or_default();
    let mut c = stem.chars();
    c.next()
        .map(|first| first.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Free bytes on the data volume.
fn disk_free(dir: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(dir).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_texts() {
        assert_eq!(clip_text(Path::new("/x/watch_your_mouth.wav")), "Watch your mouth");
        assert_eq!(clip_text(Path::new("keep_it_clean.wav")), "Keep it clean");
    }
}
