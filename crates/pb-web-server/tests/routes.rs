//! The web server end to end over HTTP: who may log in and what they see, the host allowlist, form tokens, the live
//! socket's checks, the setup wizard, and every page rendered on the server (a page that panics fails here).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use common::{ADA, BEA, G, LOUNGE, MAX, OWNER, Web};
use futures::{SinkExt, StreamExt};
use pb_domain::{GuildId, UserId};
use pb_live_proto::{ClientMsg, PROTO, ServerMsg, Topic, TopicState};
use reqwest::header::HOST;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const PAGES: &[&str] = &["/", "/voice-lines", "/reports", "/audit", "/system"];

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_visitors_get_the_login_page() {
    let w = Web::start(true).await;
    let community = format!("/c/{G}");
    for path in PAGES.iter().copied().chain([community.as_str()]) {
        let p = w.get(path, None).await;
        assert_eq!(p.status, 200, "{path}");
        assert!(p.body.contains("href=\"/login\""), "{path}: {}", p.body);
        assert!(!p.body.contains("class=\"sidebar\""), "{path} shows the app to nobody");
    }
    assert_eq!(w.get("/no/such/page", None).await.status, 404);
    let h = w.get("/healthz", None).await;
    assert_eq!(h.status, 200);
    assert!(h.body.contains("\"fluxer\":\"Ready\""), "{}", h.body);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_carry_a_strict_security_policy() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let page = w.get("/", Some(&owner)).await;
    let csp = page.csp.expect("a Content-Security-Policy header");
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|r| r.split('\'').next())
        .expect("a script nonce");
    assert!(
        page.body.contains(&format!("nonce=\"{nonce}\"")),
        "the hydration script carries the nonce"
    );
    assert!(
        csp.contains("style-src 'self';") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    assert!(!page.body.contains(" style=\""), "no inline styles");
    // Each response has its own nonce.
    let again = w.get("/", Some(&owner)).await.csp.unwrap();
    assert_ne!(csp, again);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_host_names_are_refused() {
    let w = Web::start(true).await;
    let evil = w.send(w.http.get(w.url("/")).header(HOST, "evil.example"), None).await;
    assert_eq!(evil.status, 421);
    let local = w.send(w.http.get(w.url("/")).header(HOST, "localhost:1"), None).await;
    assert_eq!(local.status, 200);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn who_may_log_in_and_what_each_login_sees() {
    let w = Web::start(true).await;
    // The owner: every community and the System page.
    let owner = w.login(OWNER).await.unwrap();
    let page = w.get("/", Some(&owner)).await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("class=\"sidebar\""));
    assert!(
        page.body.contains("Alpha") && page.body.contains("Beta"),
        "{}",
        page.body
    );
    assert!(page.body.contains("href=\"/system\""));
    // An admin through a role (Manage Community): that community only, no System page.
    let ada = w.login(ADA).await.unwrap();
    let page = w.get("/", Some(&ada)).await;
    assert!(
        page.body.contains("Alpha") && !page.body.contains("Beta"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("href=\"/system\""));
    // The owner of another community: only that one.
    let bea = w.login(BEA).await.unwrap();
    let page = w.get("/", Some(&bea)).await;
    assert!(
        page.body.contains("Beta") && !page.body.contains("Alpha"),
        "{}",
        page.body
    );
    // A member without rights is not logged in, and told why.
    let max = w.login(MAX).await.unwrap_err();
    assert!(max.contains("max is not an admin"), "{max}");
    // Logins are recorded.
    let logins = w
        .log
        .scan(1)
        .filter_map(|e| async move { e.ok().filter(|e| e.kind == "login") })
        .count()
        .await;
    assert_eq!(logins, 3);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_renders_for_every_kind_of_login() {
    let w = Web::start(true).await;
    let mut paths: Vec<String> = PAGES.iter().map(|p| (*p).to_owned()).collect();
    for tab in ["", "/voice-lines", "/settings", "/reports"] {
        paths.push(format!("/c/{G}{tab}"));
    }
    for tab in ["", "/history", "/evidence", "/voice-lines", "/settings"] {
        paths.push(format!("/c/{G}/p/{MAX}{tab}"));
    }
    for (user, alpha) in [(OWNER, true), (ADA, true), (BEA, false)] {
        let cookie = w.login(user).await.unwrap();
        for path in &paths {
            let p = w.get(path, Some(&cookie)).await;
            let in_alpha = path.starts_with("/c/");
            let expect = if (in_alpha && !alpha) || (path == "/system" && user != OWNER) {
                404
            } else {
                200
            };
            assert_eq!(p.status, expect, "{user} {path}");
            assert!(p.body.contains("</html>"), "{user} {path}: the page did not finish");
        }
    }
    // Unknown communities, people and tabs.
    let owner = w.login(OWNER).await.unwrap();
    for path in ["/c/1", "/c/x", &format!("/c/{G}/nope"), &format!("/c/{G}/p/{MAX}/nope")] {
        assert_eq!(w.get(path, Some(&owner)).await.status, 404, "{path}");
    }
    w.stop().await;
}

/// The current value of a setting at a scope, as JSON.
fn setting(w: &Web, scope: pb_domain::Scope, key: &str) -> Option<serde_json::Value> {
    w.engine
        .settings()
        .current()
        .overrides(scope)
        .get_json(key.parse().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_forms_respect_scope_and_role() {
    let w = Web::start(true).await;
    let server = pb_domain::Scope::Server {
        guild: pb_domain::GuildId(G),
    };
    let owner = w.login(OWNER).await.unwrap();
    let ada = w.login(ADA).await.unwrap();
    let bea = w.login(BEA).await.unwrap();
    let (co, ca, cb) = (
        w.page_csrf(&owner).await,
        w.page_csrf(&ada).await,
        w.page_csrf(&bea).await,
    );
    let w = &w;
    let set = async |cookie: String, token: String, scope: String, key: &str, value: &str| {
        w.post(
            "/settings",
            Some(&cookie),
            &[
                ("csrf", &token),
                ("scope", &scope),
                ("key", key),
                ("value", value),
                ("action", "set"),
                ("back", "/"),
            ],
        )
        .await
    };
    // The owner changes a global setting.
    set(owner.clone(), co.clone(), "global".into(), "threshold", "0.7").await;
    assert_eq!(
        setting(w, pb_domain::Scope::Global, "threshold"),
        Some(serde_json::json!(0.7))
    );
    // An admin of Alpha: Alpha yes, global no, owner-only settings no.
    set(ada.clone(), ca.clone(), format!("server:{G}"), "threshold", "0.6").await;
    assert_eq!(setting(w, server, "threshold"), Some(serde_json::json!(0.6)));
    set(ada.clone(), ca.clone(), "global".into(), "threshold", "0.1").await;
    assert_eq!(
        setting(w, pb_domain::Scope::Global, "threshold"),
        Some(serde_json::json!(0.7)),
        "an admin changed a global setting"
    );
    set(ada.clone(), ca.clone(), format!("server:{G}"), "modlog_audio", "on").await;
    assert_eq!(
        setting(w, server, "modlog_audio"),
        None,
        "an admin changed an owner-only setting"
    );
    // The owner of another community: not Alpha.
    set(bea.clone(), cb.clone(), format!("server:{G}"), "threshold", "0.2").await;
    assert_eq!(
        setting(w, server, "threshold"),
        Some(serde_json::json!(0.6)),
        "an outsider changed Alpha"
    );
    // A forged form changes nothing.
    set(ada.clone(), "forged".into(), format!("server:{G}"), "threshold", "0.3").await;
    assert_eq!(setting(w, server, "threshold"), Some(serde_json::json!(0.6)));
    // An invalid value is refused with the reason.
    let r = set(ada.clone(), ca.clone(), format!("server:{G}"), "threshold", "2").await;
    let n = r.cookie("pb_notice").unwrap();
    let page = w.get("/", Some(&format!("{ada}; pb_notice={n}"))).await.body;
    let at = page
        .find("role=\"status\"")
        .map(|i| &page[i.saturating_sub(120)..(i + 200).min(page.len())]);
    assert!(
        page.contains("class=\"notice error\"") && page.contains("General threshold: 2 is not between 0 and 1"),
        "{at:?}"
    );
    // A German browser gets the reason in German.
    let form = [
        ("csrf", ca.as_str()),
        ("scope", &format!("server:{G}")),
        ("key", "threshold"),
        ("value", "2"),
        ("action", "set"),
        ("back", "/"),
    ];
    let req = w
        .http
        .post(w.url("/settings"))
        .header("accept-language", "de-DE,de;q=0.9")
        .form(&form);
    let n = w.send(req, Some(&ada)).await.cookie("pb_notice").unwrap();
    let de = w.get("/", Some(&format!("{ada}; pb_notice={n}"))).await.body;
    assert!(
        de.contains("Allgemeine Schwelle: 2 liegt nicht zwischen 0 und 1"),
        "a German reason"
    );
    // "Use inherited" clears it again; the escalation table saves as steps.
    w.post(
        "/settings",
        Some(&ada),
        &[
            ("csrf", &ca),
            ("scope", &format!("server:{G}")),
            ("key", "threshold"),
            ("action", "clear"),
            ("back", "/"),
        ],
    )
    .await;
    assert_eq!(setting(w, server, "threshold"), None);
    w.post(
        "/settings",
        Some(&ada),
        &[
            ("csrf", &ca),
            ("scope", &format!("server:{G}")),
            ("key", "escalation"),
            ("action", "set"),
            ("back", "/"),
            ("esc.from", "1"),
            ("esc.action", "none"),
            ("esc.duration", ""),
            ("esc.owner", "off"),
            ("esc.modlog", "on"),
            ("esc.from", "2"),
            ("esc.action", "mute"),
            ("esc.duration", "10m"),
            ("esc.owner", "on"),
            ("esc.modlog", "on"),
            ("esc.from", ""),
            ("esc.action", "none"),
            ("esc.duration", ""),
            ("esc.owner", "off"),
            ("esc.modlog", "off"),
        ],
    )
    .await;
    let esc = setting(w, server, "escalation").expect("escalation saved");
    assert_eq!(esc.as_array().map(Vec::len), Some(2), "{esc}");
    // The settings page shows where values come from.
    let page = w.get(&format!("/c/{G}/settings"), Some(&ada)).await.body;
    assert!(page.contains("set here") && page.contains("from global"), "badges");
}

#[tokio::test(flavor = "multi_thread")]
async fn tracking_people_from_the_web() {
    let w = Web::start(true).await;
    let ada = w.login(ADA).await.unwrap();
    let csrf = Web::csrf(&w.get("/", Some(&ada)).await.body);
    let g = pb_domain::GuildId(G);
    let r = w
        .post(
            "/people/track",
            Some(&ada),
            &[
                ("csrf", &csrf),
                ("guild", &G.to_string()),
                ("user", &format!("<@{OWNER}>")),
                ("back", "/"),
            ],
        )
        .await;
    assert_eq!(r.status, 303);
    assert!(w.engine.settings().current().is_tracked(g, pb_domain::UserId(OWNER)));
    // Bea may not track people in Alpha.
    let bea = w.login(BEA).await.unwrap();
    let cb = Web::csrf(&w.get("/", Some(&bea)).await.body);
    w.post(
        "/people/untrack",
        Some(&bea),
        &[
            ("csrf", &cb),
            ("guild", &G.to_string()),
            ("user", &OWNER.to_string()),
            ("back", "/"),
        ],
    )
    .await;
    assert!(w.engine.settings().current().is_tracked(g, pb_domain::UserId(OWNER)));
    w.post(
        "/people/untrack",
        Some(&ada),
        &[
            ("csrf", &csrf),
            ("guild", &G.to_string()),
            ("user", &OWNER.to_string()),
            ("back", "/"),
        ],
    )
    .await;
    assert!(!w.engine.settings().current().is_tracked(g, pb_domain::UserId(OWNER)));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clips_uploaded_used_in_voice_lines_and_served() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = Web::csrf(&w.get("/voice-lines", Some(&owner)).await.body);
    let wav = std::fs::read(pb_testkit::fixture("benign_1.wav")).unwrap();
    let form = reqwest::multipart::Form::new()
        .text("csrf", csrf.clone())
        .text("back", "/voice-lines")
        .text("name", "Calm down")
        .text("lang", "en")
        .part("file", reqwest::multipart::Part::bytes(wav).file_name("calm.wav"));
    let r = w.send(w.http.post(w.url("/clips")).multipart(form), Some(&owner)).await;
    assert_eq!(r.status, 303, "{r:?}");
    let page = w.get("/voice-lines", Some(&owner)).await.body;
    assert!(page.contains("Calm down"), "the library lists the clip");
    let at = page.find("/media/clip/").expect("a player for the clip") + 12;
    let hash: String = page[at..].chars().take_while(|c| *c != '"').collect();
    // Served to logged-in people only, as WAV.
    let audio = w.get(&format!("/media/clip/{hash}"), Some(&owner)).await;
    assert_eq!(audio.status, 200);
    assert!(audio.body.starts_with("RIFF"));
    assert_eq!(w.get(&format!("/media/clip/{hash}"), None).await.status, 401);
    // The clip becomes Alpha's greeting.
    let scope = format!("server:{G}");
    let r = w
        .post(
            "/voice-lines",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("scope", &scope),
                ("line", "greeting"),
                ("op", "add_clip"),
                ("clip", &hash),
                ("back", "/"),
            ],
        )
        .await;
    assert_eq!(r.status, 303);
    let tree = w.engine.settings().current();
    let slot = tree
        .voice_lines(pb_domain::Scope::Server {
            guild: pb_domain::GuildId(G),
        })
        .and_then(|s| s.get(&"greeting".parse().unwrap()))
        .cloned();
    assert_eq!(slot.map(|s| s.clips.len()), Some(1));
    // Text for a warning step at Alpha, then the preview of the greeting plays the clip.
    w.post(
        "/voice-lines",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &scope),
            ("line_kind", "warning"),
            ("line_label", "profanity"),
            ("line_step", "2"),
            ("op", "set_text"),
            ("lang", "en"),
            ("text", "{name}, twice now."),
            ("back", "/"),
        ],
    )
    .await;
    assert!(
        w.get(&format!("/c/{G}/voice-lines"), Some(&owner))
            .await
            .body
            .contains("{name}, twice now.")
    );
    let preview = w
        .get(&format!("/media/preview?scope={scope}&line=greeting"), Some(&owner))
        .await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    // Bea may not change Alpha's lines.
    let bea = w.login(BEA).await.unwrap();
    let cb = Web::csrf(&w.get("/", Some(&bea)).await.body);
    w.post(
        "/voice-lines",
        Some(&bea),
        &[
            ("csrf", &cb),
            ("scope", &scope),
            ("line", "greeting"),
            ("op", "clear"),
            ("back", "/"),
        ],
    )
    .await;
    assert!(
        w.engine
            .settings()
            .current()
            .voice_lines(pb_domain::Scope::Server {
                guild: pb_domain::GuildId(G)
            })
            .and_then(|s| s.get(&"greeting".parse().unwrap()))
            .is_some()
    );
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn logging_out_needs_the_form_token() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = Web::csrf(&w.get("/", Some(&owner)).await.body);
    let forged = w.post("/auth/logout", Some(&owner), &[("csrf", "forged")]).await;
    assert_eq!(forged.status, 303);
    assert!(
        w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""),
        "a forged form logged out"
    );
    let real = w.post("/auth/logout", Some(&owner), &[("csrf", &csrf)]).await;
    assert_eq!(real.status, 303);
    assert!(
        !w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""),
        "still logged in"
    );
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_started_elsewhere_is_not_finished_here() {
    let w = Web::start(true).await;
    w.fake.log_in_browser(Some(OWNER));
    let start = w.get("/login", None).await;
    let at_fluxer = w.get(start.location.as_deref().unwrap(), None).await;
    // Coming back without the browser's login cookie (another browser, or a forged link).
    let back = w.get(at_fluxer.location.as_deref().unwrap(), None).await;
    assert_eq!(back.status, 303);
    assert!(back.cookie("pb_session").is_none());
    w.stop().await;
}

async fn connect(
    w: &Web,
    origin: Option<&str>,
    cookie: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let mut req = (w.base.replace("http://", "ws://") + "/live")
        .into_client_request()
        .unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(ws, _)| ws)
}

async fn next_msg(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> ServerMsg {
    loop {
        let m = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .expect("a message in time")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_live_socket_checks_origin_and_login() {
    let w = Web::start(true).await;
    assert!(connect(&w, None, None).await.is_err(), "no Origin");
    assert!(
        connect(&w, Some("http://evil.example"), None).await.is_err(),
        "another site's page"
    );
    let mut anon = connect(&w, Some(&w.base), None).await.unwrap();
    assert_eq!(next_msg(&mut anon).await, ServerMsg::AuthExpired);

    let ada = w.login(ADA).await.unwrap();
    let mut ws = connect(&w, Some(&w.base), Some(&ada)).await.unwrap();
    let hello = ClientMsg::Hello {
        proto: PROTO,
        view: 1,
        visible: true,
        topics: vec![Topic::Sidebar, Topic::System],
    };
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
        .await
        .unwrap();
    let mut saw_sidebar = false;
    let mut denied_system = false;
    while !(saw_sidebar && denied_system) {
        match next_msg(&mut ws).await {
            ServerMsg::Snapshot {
                topic: Topic::Sidebar,
                state,
                ..
            } => {
                let TopicState::Sidebar(s) = *state else {
                    panic!("a sidebar")
                };
                let names: Vec<_> = s.communities.iter().map(|c| c.name.clone()).collect();
                assert_eq!(names, ["Alpha"], "an admin of Alpha sees only Alpha");
                saw_sidebar = true;
            }
            ServerMsg::Denied {
                topic: Topic::System, ..
            } => denied_system = true,
            ServerMsg::Snapshot {
                topic: Topic::System, ..
            } => panic!("an admin got the owner-only System topic"),
            _ => {}
        }
    }
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_community_view_shows_who_is_muted_or_deafened() {
    let w = Web::start(true).await;
    w.wait_ready().await;
    let conn = w.fake.voice_join(G, LOUNGE, MAX);
    w.fake.voice_self(&conn, true, true);
    let ada = w.login(ADA).await.unwrap();
    let hello = ClientMsg::Hello {
        proto: PROTO,
        view: 1,
        visible: true,
        topics: vec![Topic::Guild { guild: GuildId(G) }],
    };
    // The voice update reaches the bot through the gateway; a fresh page shows it once it has.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let mut ws = connect(&w, Some(&w.base), Some(&ada)).await.unwrap();
        ws.send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
            .await
            .unwrap();
        let state = loop {
            if let ServerMsg::Snapshot {
                topic: Topic::Guild { .. },
                state,
                ..
            } = next_msg(&mut ws).await
            {
                let TopicState::Guild(g) = *state else {
                    panic!("a community")
                };
                break g;
            }
        };
        let max = state
            .calls
            .iter()
            .flat_map(|c| &c.participants)
            .find(|p| p.who.user == UserId(MAX));
        if let Some(p) = max.filter(|p| p.muted && p.deaf) {
            assert!(!p.bot);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "Max shows muted and deafened");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_setup_wizard_from_code_to_owner() {
    let w = Web::start(false).await;
    // Before setup, logging in leads to the wizard.
    assert_eq!(w.get("/login", None).await.location.as_deref(), Some("/setup"));
    let page = w.get("/setup", None).await;
    assert!(page.body.contains("Setup code"), "{}", page.body);
    let code = std::fs::read_to_string(w.dir.path().join("setup-code")).unwrap();
    // A wrong code: told so, and the next try has to wait.
    let wrong = w
        .post("/setup", None, &[("step", "code"), ("value", "AAAA-AAAA")])
        .await;
    let notice = wrong.cookie("pb_notice").unwrap();
    assert!(
        w.get("/setup", Some(&format!("pb_notice={notice}")))
            .await
            .body
            .contains("not the setup code")
    );
    // The right code (typed in lower case with a space) opens a wizard session.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let typed = code.trim().to_lowercase().replace('-', " ");
    let ok = w.post("/setup", None, &[("step", "code"), ("value", &typed)]).await;
    let setup = format!("pb_setup={}", ok.cookie("pb_setup").expect("a wizard session"));
    let page = w.get("/setup", Some(&setup)).await;
    assert!(page.body.contains("Instance address"), "{}", page.body);
    let csrf = Web::csrf(&page.body);
    // A form without the wizard's token is refused.
    let forged = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "instance"), ("csrf", "x"), ("value", &w.fake.url())],
        )
        .await;
    assert!(forged.cookie("pb_notice").is_some());
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "instance"), ("csrf", &csrf), ("value", &w.fake.url())],
        )
        .await;
    assert!(r.cookie("pb_notice").is_none(), "{r:?}");
    // A token that is not a bot token, then a rejected one, then the right one.
    let page = w.get("/setup", Some(&setup)).await;
    assert!(page.body.contains("Bot token"), "{}", page.body);
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "token"), ("csrf", &csrf), ("value", "nonsense")],
        )
        .await;
    assert!(r.cookie("pb_notice").is_some());
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "token"), ("csrf", &csrf), ("value", "1000.wrong")],
        )
        .await;
    let n = r.cookie("pb_notice").expect("a rejected token is reported");
    assert!(
        w.get("/setup", Some(&format!("{setup}; pb_notice={n}")))
            .await
            .body
            .contains("rejected this token")
    );
    let token = w.fake.config().token.clone();
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "token"), ("csrf", &csrf), ("value", &token)],
        )
        .await;
    assert!(r.cookie("pb_notice").is_none(), "{r:?}");
    // The client secret: the page shows the redirect address to register.
    let page = w.get("/setup", Some(&setup)).await;
    assert!(
        page.body.contains(&format!("{}/auth/callback", w.base)),
        "{}",
        page.body
    );
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "secret"), ("csrf", &csrf), ("value", "a-mistyped-secret")],
        )
        .await;
    assert!(r.cookie("pb_notice").is_none(), "{r:?}");
    // The owner logs in; with a mistyped secret Fluxer refuses, and the secret can be entered again.
    let page = w.get("/setup", Some(&setup)).await;
    assert!(page.body.contains("The bot is online as watchbot"), "{}", page.body);
    assert!(w.login_with(OWNER, Some(&setup)).await.is_err());
    let page = w.get("/setup", Some(&setup)).await;
    assert!(page.body.contains("Enter the client secret again"), "{}", page.body);
    let secret = w.fake.config().client_secret.clone();
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "secret"), ("csrf", &csrf), ("value", &secret)],
        )
        .await;
    assert!(r.cookie("pb_notice").is_none(), "{r:?}");
    // Setup is finished and the code is gone.
    let owner = w.login_with(OWNER, Some(&setup)).await.unwrap();
    assert!(w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""));
    assert!(!w.dir.path().join("setup-code").exists());
    assert!(w.get("/setup", None).await.body.contains("Setup is finished"));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_overview_checks_permissions_and_the_sidebar_links_the_invite() {
    let w = Web::start(true).await;
    let ada = w.login(ADA).await.unwrap();
    let page = w.get(&format!("/c/{G}"), Some(&ada)).await.body;
    assert!(
        page.contains("🔊 Lounge") && page.contains("all it needs"),
        "voice channel checked"
    );
    let bot = pb_fluxer_api::perms::BOT;
    assert!(
        page.contains(&format!(
            "/oauth2/authorize?client_id=1000&amp;scope=bot&amp;permissions={bot}"
        )),
        "invite link"
    );
    // With moderation actions on, the missing member permissions show, and the invite asks for them.
    // (On everywhere, so the invite asks for them too.)
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let r = w
        .post(
            "/settings",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("scope", "global"),
                ("key", "actions_enabled"),
                ("value", "on"),
                ("action", "set"),
                ("back", "/"),
            ],
        )
        .await;
    assert_eq!(r.status, 303);
    let page = w.get(&format!("/c/{G}"), Some(&ada)).await.body;
    assert!(
        page.contains("missing: Mute members, Move members, Time out members"),
        "member actions checked"
    );
    let all = bot
        | pb_fluxer_api::perms::MUTE_MEMBERS
        | pb_fluxer_api::perms::MOVE_MEMBERS
        | pb_fluxer_api::perms::MODERATE_MEMBERS;
    assert!(page.contains(&format!("permissions={all}")), "invite with actions");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn admins_rules_as_in_the_old_bot() {
    let w = Web::start(true).await;
    let g = pb_domain::GuildId(G);
    let server = format!("server:{G}");
    let owner = w.login(OWNER).await.unwrap();
    let co = w.page_csrf(&owner).await;
    let ada = w.login(ADA).await.unwrap();
    let ca = w.page_csrf(&ada).await;
    let setting = |key: &str| {
        w.engine
            .settings()
            .current()
            .overrides(pb_domain::Scope::Server { guild: g })
            .get_json(key.parse().unwrap())
    };
    let form = |token: &str, key: &str, action: &str| {
        vec![
            ("csrf", token.to_owned()),
            ("scope", server.clone()),
            ("key", key.to_owned()),
            ("value", "on".to_owned()),
            ("action", action.to_owned()),
            ("back", "/".to_owned()),
        ]
    };
    let post = async |cookie: &str, f: Vec<(&str, String)>| {
        let f: Vec<(&str, &str)> = f.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.post("/settings", Some(cookie), &f).await
    };
    // What only the owner may set, an admin may not clear either.
    post(&owner, form(&co, "modlog_audio", "set")).await;
    assert_eq!(setting("modlog_audio"), Some(serde_json::json!(true)));
    post(&ada, form(&ca, "modlog_audio", "clear")).await;
    assert_eq!(
        setting("modlog_audio"),
        Some(serde_json::json!(true)),
        "an admin cleared an owner-only setting"
    );

    // The bot does not track itself.
    let bot = w.fake.config().bot_id.to_string();
    let r = w
        .post(
            "/people/track",
            Some(&ada),
            &[("csrf", &ca), ("guild", &G.to_string()), ("user", &bot), ("back", "/")],
        )
        .await;
    assert_eq!(r.status, 303);
    assert!(
        !w.engine
            .settings()
            .current()
            .listed_for(g)
            .contains(&pb_domain::UserId(w.fake.config().bot_id))
    );

    // A clip in the shared library: only who added it (or the owner) changes it.
    let wav = std::fs::read(pb_testkit::fixture("benign_1.wav")).unwrap();
    let upload = reqwest::multipart::Form::new()
        .text("csrf", co.clone())
        .text("back", "/voice-lines")
        .text("name", "Owner's clip")
        .part("file", reqwest::multipart::Part::bytes(wav).file_name("c.wav"));
    w.send(w.http.post(w.url("/clips")).multipart(upload), Some(&owner))
        .await;
    let page = w.get("/voice-lines", Some(&ada)).await.body;
    assert!(
        page.contains("Owner&#x27;s clip") || page.contains("Owner's clip"),
        "Ada sees the clip"
    );
    assert!(!page.contains("action=\"/clips/remove\""), "but cannot remove it");
    let at = page.find("/media/clip/").unwrap() + 12;
    let hash: String = page[at..].chars().take_while(|c| *c != '"').collect();
    w.post(
        "/clips/remove",
        Some(&ada),
        &[("csrf", &ca), ("clip", &hash), ("back", "/voice-lines")],
    )
    .await;
    assert!(
        w.get("/voice-lines", Some(&owner)).await.body.contains("Owner"),
        "the clip is still there"
    );

    // Someone tracked in a community never manages the bot there (Ada loses Alpha).
    w.engine
        .track(g, &[pb_domain::UserId(ADA)], pb_store_api::Actor::system())
        .await
        .unwrap();
    let err = w.login(ADA).await.unwrap_err();
    assert!(err.contains("not an admin"), "{err}");

    // The audit page reads every entry, filtered by kind and community.
    let audit = w
        .get(&format!("/audit?kind=settings&community={G}"), Some(&owner))
        .await;
    assert_eq!(audit.status, 200);
    assert!(
        audit.body.contains("Audio in the mod log"),
        "the setting change, by name: {}",
        audit.body
    );
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_to_an_unregistered_address_is_explained() {
    let w = Web::start(true).await;
    w.fake.log_in_browser(Some(OWNER));
    // The same web UI by another name: Fluxer would not send the browser back to it.
    let port = w.base.rsplit(':').next().unwrap().to_owned();
    let start = w
        .send(
            w.http.get(w.url("/login")).header(HOST, format!("localhost:{port}")),
            None,
        )
        .await;
    assert_eq!(start.status, 303);
    let notice = start.cookie("pb_notice").expect("a notice instead of a trip to Fluxer");
    let page = w.get("/", Some(&format!("pb_notice={notice}"))).await.body;
    assert!(
        page.contains(&format!("http://localhost:{port}/auth/callback")) && page.contains("Redirect URIs"),
        "{page}"
    );
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn serves_https_with_secure_cookies() {
    let w = Web::start_with(true, true).await;
    assert!(w.base.starts_with("https://"));
    let health = w.get("/healthz", None).await;
    assert_eq!(health.status, 200, "{health:?}");
    // A login over HTTPS gets a cookie only sent over HTTPS.
    w.fake.log_in_browser(Some(OWNER));
    let start = w.send(w.http.get(w.url("/login")), None).await;
    assert_eq!(start.status, 303);
    let set = start.cookies.join("\n");
    assert!(set.contains("pb_oauth=") && set.contains("Secure"), "{set}");
    let owner = w.login(OWNER).await.unwrap();
    assert_eq!(w.get("/", Some(&owner)).await.status, 200);
    w.stop().await;
}
