//! The web UI in a real browser (Chromium over CDP) against the running `pb` binary, a fake Fluxer, a local LiveKit
//! server and two people talking: the login, the live wall (levels, the conveyor), two tabs with one frozen for 30 s
//! (the old bot froze here), and the end of a login while a tab is open.
//!
//! Needs livekit-server, the model weights (PB_WEIGHTS) and the built site (`cargo xtask web`).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::network::{EnableParams, EventWebSocketCreated};
use chromiumoxide::cdp::browser_protocol::page::{SetWebLifecycleStateParams, SetWebLifecycleStateState};
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use pb_devstack::{ALICE, Opts, Stack, workspace_root};

/// `pb run` as a child process; stopped with SIGTERM.
struct Bot {
    child: Child,
}

impl Bot {
    fn stop(mut self) -> Option<i32> {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status();
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if let Ok(Some(s)) = self.child.try_wait() {
                return s.code();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        None
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// What a page wrote to its console and the exceptions it threw (shown when a check times out).
type Console = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

static CONSOLE: std::sync::LazyLock<Console> = std::sync::LazyLock::new(Console::default);

/// The bot's log file (its end is shown when a check times out).
static BOT_LOG: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

fn bot_log_tail() -> String {
    let text = BOT_LOG
        .get()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(30)..].join("\n")
}

async fn watch_console(page: &Page) {
    use chromiumoxide::cdp::js_protocol::runtime::{
        EnableParams as RuntimeEnable, EventConsoleApiCalled, EventExceptionThrown,
    };
    page.execute(RuntimeEnable::default()).await.unwrap();
    let mut logs = page.event_listener::<EventConsoleApiCalled>().await.unwrap();
    let mut errors = page.event_listener::<EventExceptionThrown>().await.unwrap();
    tokio::spawn(async move {
        loop {
            let line = tokio::select! {
                Some(e) = logs.next() => format!(
                    "console.{:?}: {}",
                    e.r#type,
                    e.args.iter().filter_map(|a| a.value.as_ref().map(ToString::to_string).or_else(|| a.description.clone())).collect::<Vec<_>>().join(" ")
                ),
                Some(e) = errors.next() => format!("exception: {}", e.exception_details.exception.as_ref().and_then(|x| x.description.clone()).unwrap_or_else(|| e.exception_details.text.clone())),
                else => break,
            };
            CONSOLE.lock().unwrap().push(line);
        }
    });
}

/// Runs a script in the page; while the page is navigating (no context to run in) it is tried again.
async fn eval<T: serde::de::DeserializeOwned>(page: &Page, js: &str) -> T {
    let mut last = None;
    for _ in 0..50 {
        match page.evaluate(js).await {
            Ok(v) => return v.into_value::<T>().unwrap(),
            Err(e) => last = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the page did not run the script: {last:?}");
}

async fn until(what: &str, secs: u64, mut f: impl AsyncFnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f().await {
        assert!(
            Instant::now() < end,
            "timed out: {what}\nbrowser console:\n{}\nbot log:\n{}",
            CONSOLE.lock().unwrap().join("\n"),
            bot_log_tail()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Each tile's person and the sentence its card shows, e.g. `[["444444","01a1…"],…]`.
const CARDS: &str = "JSON.stringify([...document.querySelectorAll('.tile')].map(t => \
    [t.dataset.user, t.querySelector('.station')?.dataset.card || ''])) ";

/// The newest change to a source file under `dir`.
fn newest_source(dir: &std::path::Path) -> std::time::SystemTime {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let t = if p.is_dir() {
            newest_source(&p)
        } else {
            e.metadata().and_then(|m| m.modified()).unwrap_or(newest)
        };
        newest = newest.max(t);
    }
    newest
}

/// One rig at a time: each runs the bot with its models, a LiveKit server and a browser, and the timing checks need
/// the machine to themselves.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The stack, the bot and a browser.
struct Rig {
    _serial: tokio::sync::MutexGuard<'static, ()>,
    ui: String,
    dir: tempfile::TempDir,
    stack: Stack,
    bot: Bot,
    browser: Browser,
    pump: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let serial = SERIAL.lock().await;
        let _ = tracing_subscriber::fmt()
            .with_env_filter("warn")
            .with_test_writer()
            .try_init();
        pb_tls::install_default().unwrap();
        let root = workspace_root();
        // The bundle must come from the same sources as the server (island names carry source positions).
        let built = std::fs::metadata(root.join("target/site/pkg/pb_bg.wasm"))
            .and_then(|m| m.modified())
            .expect("build the browser bundle first: cargo xtask web");
        let newest = newest_source(&root.join("crates/pb-web/src"));
        assert!(
            newest <= built,
            "the browser bundle is older than crates/pb-web/src: run cargo xtask web"
        );
        assert!(pb_testkit::lk::available(), "livekit-server is needed (LIVEKIT_SERVER)");
        let dir = tempfile::tempdir().unwrap();
        let port = pb_testkit::lk::free_port().unwrap();
        let ui = format!("http://127.0.0.1:{port}");
        let opts = Opts {
            data: dir.path().join("data"),
            fluxer: "127.0.0.1:0".parse().unwrap(),
            web: format!("127.0.0.1:{port}").parse().unwrap(),
            ui: ui.clone(),
            weights: pb_testkit::weights(),
            ready: true,
            talk: true,
        };
        let stack = Stack::start(&opts).await.unwrap();
        let log = std::fs::File::create(dir.path().join("pb.log")).unwrap();
        let _ = BOT_LOG.set(dir.path().join("pb.log"));
        let bot = Bot {
            child: Command::new(env!("CARGO_BIN_EXE_pb"))
                .arg("run")
                .env("PB_DATA", &opts.data)
                .env("RUST_LOG", "warn,pb_web_server=debug,tower_http=debug")
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        };
        let http = reqwest::Client::new();
        until("the bot serves and is logged in to Fluxer", 120, async || {
            let r = http.get(format!("{ui}/healthz")).send().await;
            match r {
                Ok(r) => r.text().await.unwrap_or_default().contains("\"fluxer\":\"Ready\""),
                Err(_) => false,
            }
        })
        .await;
        let (browser, mut handler) = Browser::launch(
            BrowserConfig::builder()
                .chrome_executable(std::env::var("PB_CHROMIUM").unwrap_or_else(|_| "chromium".into()))
                .no_sandbox()
                .window_size(1280, 900)
                .user_data_dir(dir.path().join("chromium"))
                // A fake microphone, allowed without asking (recording clips).
                .arg("use-fake-ui-for-media-stream")
                .arg("use-fake-device-for-media-stream")
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
        let pump = tokio::spawn(async move { while handler.next().await.is_some() {} });
        Rig {
            _serial: serial,
            ui,
            dir,
            stack,
            bot,
            browser,
            pump,
        }
    }

    async fn stop(mut self) {
        self.browser.close().await.unwrap();
        let _ = self.browser.wait().await;
        self.pump.abort();
        assert_eq!(self.bot.stop(), Some(0), "pb run stops cleanly on SIGTERM");
        drop(self.stack);
        drop(self.dir);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs chromium (PB_CHROMIUM), LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn live_wall_two_tabs_frozen_tab_and_ended_login() {
    let rig = Rig::start().await;
    let (ui, browser) = (rig.ui.clone(), &rig.browser);
    // Tab A logs in (the fake Fluxer approves at once) and lands on the wall.
    let a = browser.new_page(format!("{ui}/login")).await.unwrap();
    until("tab A shows Alice's tile", 60, async || {
        eval::<bool>(&a, &format!("!!document.querySelector('.tile[data-user=\"{ALICE}\"]')")).await
    })
    .await;
    // Live levels arrive (the page hydrated and subscribed).
    until("the level sparkline fills", 30, async || {
        eval::<bool>(
            &a,
            "[...document.querySelectorAll('.spark path')].some(p => (p.getAttribute('d') || '').length > 0)",
        )
        .await
    })
    .await;
    // The conveyor moves: Alice's card goes through more than one station.
    let mut stations = std::collections::BTreeSet::new();
    until("Alice's card moves along the conveyor", 40, async || {
        let s: String = eval(
            &a,
            &format!("document.querySelector('.tile[data-user=\"{ALICE}\"] .station')?.dataset.station || ''"),
        )
        .await;
        if !s.is_empty() {
            stations.insert(s);
        }
        stations.len() >= 2
    })
    .await;

    // Tab B, the same login.
    let b = browser.new_page(format!("{ui}/")).await.unwrap();
    until("tab B shows the wall", 30, async || {
        eval::<bool>(&b, "!!document.querySelector('.tile')").await
    })
    .await;
    a.execute(EnableParams::default()).await.unwrap();
    let mut sockets = a.event_listener::<EventWebSocketCreated>().await.unwrap();

    // A is frozen for 30 s while B is in front and new sentences keep coming.
    a.execute(SetWebLifecycleStateParams::new(SetWebLifecycleStateState::Frozen))
        .await
        .unwrap();
    b.bring_to_front().await.unwrap();
    tokio::time::sleep(Duration::from_secs(30)).await;
    let before: String = eval(&b, CARDS).await;
    a.execute(SetWebLifecycleStateParams::new(SetWebLifecycleStateState::Active))
        .await
        .unwrap();
    a.bring_to_front().await.unwrap();
    // Within 2 s A shows what B shows (cards may move on in between: compare to B at the same moment).
    let resumed = Instant::now();
    loop {
        let (sa, sb): (String, String) = (eval(&a, CARDS).await, eval(&b, CARDS).await);
        if sa == sb && sa != "[]" {
            break;
        }
        assert!(
            resumed.elapsed() < Duration::from_secs(2),
            "tab A did not catch up within 2 s after 30 s frozen:\nA {sa}\nB {sb}\n(B before resume {before})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // At most one new connection for the resume.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut reconnects = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), sockets.next()).await {
        reconnects += 1;
    }
    assert!(reconnects <= 1, "tab A opened {reconnects} connections after resuming");

    // B logs out: A shows "log in again" and does not keep reconnecting.
    eval::<bool>(
        &b,
        "document.querySelector('form[action=\"/auth/logout\"]').submit(), true",
    )
    .await;
    until("tab A shows that the login ended", 15, async || {
        eval::<bool>(&a, "document.body.classList.contains('auth-expired')").await
    })
    .await;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), sockets.next()).await {}
    tokio::time::sleep(Duration::from_secs(8)).await;
    let mut after = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), sockets.next()).await {
        after += 1;
    }
    assert!(
        after <= 1,
        "tab A kept reconnecting after the login ended ({after} connections in 8 s)"
    );

    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs chromium (PB_CHROMIUM), LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn people_clips_recording_and_previews() {
    let rig = Rig::start().await;
    let (ui, browser) = (rig.ui.clone(), &rig.browser);
    let g = pb_devstack::GUILD;
    let page = browser.new_page("about:blank").await.unwrap();
    watch_console(&page).await;
    page.goto(format!("{ui}/login?next=/c/{g}")).await.unwrap();
    until("the community page", 60, async || {
        eval::<bool>(&page, "!!document.querySelector('.picker input')").await
    })
    .await;
    // Typing a name lists members (after a short pause in typing); picking one tracks them.
    until("the member box is live", 30, async || {
        eval::<bool>(&page, "document.querySelector('.picker')?.dataset.ready === 'true'").await
    })
    .await;
    eval::<bool>(
        &page,
        "(() => { const i = document.querySelector('.picker input[name=user]'); i.value = 'The'; \
          i.dispatchEvent(new Event('input', {bubbles: true})); return true; })()",
    )
    .await;
    until("members are suggested", 30, async || {
        eval::<bool>(&page, "!!document.querySelector('.suggest button')").await
    })
    .await;
    eval::<bool>(&page, "document.querySelector('.suggest button').click(), true").await;
    until("The Owner is tracked", 30, async || {
        eval::<bool>(
            &page,
            "[...document.querySelectorAll('.people-list .name')].some(a => a.textContent.includes('The Owner'))",
        )
        .await
    })
    .await;

    // Upload a clip.
    let lib = browser.new_page("about:blank").await.unwrap();
    watch_console(&lib).await;
    lib.goto(format!("{ui}/voice-lines")).await.unwrap();
    until("the clip library", 30, async || {
        eval::<bool>(&lib, "!!document.querySelector('form.upload')").await
    })
    .await;
    let input = lib.find_element("form.upload input[type=file]").await.unwrap();
    let fixture = pb_testkit::fixture("benign_1.wav").display().to_string();
    lib.execute(SetFileInputFilesParams {
        files: vec![fixture],
        node_id: Some(input.node_id),
        backend_node_id: None,
        object_id: None,
    })
    .await
    .unwrap();
    eval::<bool>(&lib, "document.querySelector('form.upload input[name=name]').value = 'Calm down', document.querySelector('form.upload').submit(), true").await;
    until("the uploaded clip is listed", 60, async || {
        eval::<bool>(
            &lib,
            "[...document.querySelectorAll('table.clips input[name=name]')].some(i => i.value === 'Calm down')",
        )
        .await
    })
    .await;
    // Record one with the (fake) microphone.
    until("the recorder is live", 30, async || {
        eval::<bool>(&lib, "document.querySelector('.recorder')?.dataset.ready === 'true'").await
    })
    .await;
    eval::<bool>(&lib, "document.querySelector('.recorder button').click(), true").await;
    until("recording", 15, async || {
        eval::<bool>(&lib, "!!document.querySelector('.recorder button.recording')").await
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    eval::<bool>(&lib, "document.querySelector('.recorder button').click(), true").await;
    until("the recording is uploaded and listed", 60, async || {
        let n = eval::<u32>(&lib, "document.querySelectorAll('table.clips tr').length").await;
        if n < 2 {
            let state: String = eval(&lib, "(document.querySelector('.recorder') || {}).outerHTML || ''").await;
            let mut c = CONSOLE.lock().unwrap();
            if c.last() != Some(&state) {
                c.push(state);
            }
        }
        n >= 2
    })
    .await;
    // A voice line's preview plays (the bot renders it).
    let ok: bool = eval(
        &lib,
        &format!("fetch('/media/preview?scope=server:{g}&line=greeting').then(r => r.ok && r.headers.get('content-type') === 'audio/wav')"),
    )
    .await;
    assert!(ok, "the greeting preview");
    rig.stop().await;
}
