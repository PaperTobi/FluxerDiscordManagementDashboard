//! The web server end to end over HTTP: who may log in and what they see, the host allowlist, form tokens, the live
//! socket's checks, the setup wizard, and every page rendered on the server (a page that panics fails here).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use common::{ADA, BEA, G, G2, LOUNGE, MAX, OWNER, Web};
use futures::{SinkExt, StreamExt};
use pb_domain::{GuildId, UserId};
use pb_live_proto::{ClientMsg, PROTO, ServerMsg, Topic, TopicState};
use reqwest::header::HOST;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const PAGES: &[&str] = &[
    "/",
    "/voice-lines",
    "/reports",
    "/audit",
    "/system",
    "/settings",
    "/settings/detection",
];

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_visitors_get_the_login_page() {
    let w = Web::start(true).await;
    let community = format!("/c/{G}");
    for path in PAGES.iter().copied().chain([community.as_str()]) {
        let p = w.get(path, None).await;
        assert_eq!(p.status, 200, "{path}");
        assert!(p.body.contains("href=\"/login"), "{path}: {}", p.body);
        assert!(!p.body.contains("class=\"sidebar\""), "{path} shows the app to nobody");
    }
    // Logging in comes back to the page asked for.
    let p = w.get(&format!("{community}?before=5"), None).await;
    assert!(
        p.body
            .contains(&format!("href=\"/login?next=%2Fc%2F{G}%3Fbefore%3D5\"")),
        "{}",
        p.body
    );
    assert_eq!(w.get("/no/such/page", None).await.status, 404);
    let h = w.get("/healthz", None).await;
    assert_eq!(h.status, 200);
    assert!(h.body.contains("\"fluxer\":\"Ready\""), "{}", h.body);
    assert!(h.body.contains("\"status\":\"ok\""), "{}", h.body);
    for part in ["moderation", "undo", "digest", "gateway", "views"] {
        assert!(h.body.contains(&format!("\"name\":\"{part}\"")), "{part}: {}", h.body);
    }
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
    for tab in ["", "/voice-lines", "/settings", "/settings/detection", "/reports"] {
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
            let owners_only = path == "/system" || path.starts_with("/settings");
            let expect = if (in_alpha && !alpha) || (owners_only && user != OWNER) {
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
    // The settings pages show where values come from.
    let page = w.get(&format!("/c/{G}/settings/escalation"), Some(&ada)).await.body;
    assert!(page.contains("Changed for Alpha"), "where values come from");
    let page = w.get(&format!("/c/{G}/settings/detection"), Some(&ada)).await.body;
    assert!(page.contains("Same as for every community"), "where values come from");
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
    // Stopping asks first: the form goes to a page that says what happens, and only that page's form untracks.
    let untrack = [
        ("csrf", csrf.as_str()),
        ("guild", &G.to_string()),
        ("user", &OWNER.to_string()),
        ("back", &format!("/c/{G}")),
    ];
    let r = w.post("/people/untrack", Some(&ada), &untrack).await;
    let to = r.location.clone().unwrap();
    assert!(
        to.starts_with("/confirm?what=untrack&guild=111111&user=1002&back=%2Fc%2F111111"),
        "{to}"
    );
    assert!(!to.contains("csrf"), "the form token stays out of the address");
    assert!(w.engine.settings().current().is_tracked(g, pb_domain::UserId(OWNER)));
    let page = w.get(&to, Some(&ada)).await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Stop tracking") && page.body.contains("name=\"confirm\" value=\"1\""));
    assert!(page.body.contains(&format!("href=\"/c/{G}\"")), "Cancel goes back");
    let confirmed: Vec<(&str, &str)> = untrack.iter().copied().chain([("confirm", "1")]).collect();
    let r = w.post("/people/untrack", Some(&ada), &confirmed).await;
    assert_eq!(r.location.as_deref(), Some(format!("/c/{G}").as_str()));
    assert!(!w.engine.settings().current().is_tracked(g, pb_domain::UserId(OWNER)));
    // Bea cannot get a confirmation page for Alpha either.
    assert_eq!(w.get(&to, Some(&bea)).await.status, 404);
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
    // An address that does not answer: told so, and what was typed stays in the field.
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[
                ("step", "instance"),
                ("csrf", &csrf),
                ("value", "http://127.0.0.1:9/api"),
            ],
        )
        .await;
    assert!(r.cookie("pb_notice").is_some(), "{r:?}");
    let page = w.get("/setup", Some(&setup)).await.body;
    assert!(page.contains("value=\"http://127.0.0.1:9/api\""), "{page}");
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
    assert!(page.body.contains("The token is checked at"), "{}", page.body);
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
    // A done step can be opened again: the client secret, kept as it is …
    let step = |name: &'static str, value: &'static str| [("step", name), ("csrf", csrf.as_str()), ("value", value)];
    w.post("/setup", Some(&setup), &step("goto", "secret")).await;
    let page = w.get("/setup", Some(&setup)).await.body;
    assert!(page.contains("A client secret is saved"), "{page}");
    w.post("/setup", Some(&setup), &step("keep", "")).await;
    assert!(w.get("/setup", Some(&setup)).await.body.contains("becomes the owner"));
    // … and the instance, given as an API address with a path (its discovery document is at the server's root).
    w.post("/setup", Some(&setup), &step("goto", "instance")).await;
    assert!(w.get("/setup", Some(&setup)).await.body.contains("Instance address"));
    let with_path = format!("{}/api", w.fake.url());
    let r = w
        .post(
            "/setup",
            Some(&setup),
            &[("step", "instance"), ("csrf", &csrf), ("value", &with_path)],
        )
        .await;
    assert!(r.cookie("pb_notice").is_none(), "{r:?}");
    let page = w.get("/setup", Some(&setup)).await.body;
    assert!(page.contains("becomes the owner"), "{page}");
    // Setup is finished and the code is gone.
    let owner = w.login_with(OWNER, Some(&setup)).await.unwrap();
    assert!(w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""));
    assert!(!w.dir.path().join("setup-code").exists());
    assert!(w.get("/setup", None).await.body.contains("Setup is finished"));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_overview_checks_permissions_and_the_invite_page_asks_for_them() {
    let w = Web::start(true).await;
    let ada = w.login(ADA).await.unwrap();
    let page = w.get(&format!("/c/{G}"), Some(&ada)).await.body;
    assert!(
        page.contains("🔊 Lounge") && page.contains("all it needs"),
        "voice channel checked"
    );
    assert!(
        page.contains("href=\"/invite\""),
        "the sidebar leads to the invite page"
    );
    assert!(!page.contains("guild_id="), "nothing to authorise again");
    let bot = pb_fluxer_api::perms::BOT;
    let link = |perms: u64| format!("/oauth2/authorize?client_id=1000&amp;scope=bot&amp;permissions={perms}");
    let invite = w.get("/invite", Some(&ada)).await;
    assert_eq!(invite.status, 200);
    assert!(invite.body.contains(&link(bot)), "invite link: {}", invite.body);
    assert!(invite.body.contains("What inviting does") && invite.body.contains("Copy"));
    assert!(!invite.body.contains("not connected"), "the bot is connected");
    // With moderation actions on in Alpha only, its missing member permissions show, the invite asks for them, and
    // Alpha gets a link that authorises the bot there again.
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let r = w
        .post(
            "/settings",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("scope", &format!("server:{G}")),
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
    let again = format!("{}&amp;guild_id={G}", link(all));
    assert!(page.contains(&again), "authorise again from the overview");
    let invite = w.get("/invite", Some(&ada)).await.body;
    assert!(invite.contains(&format!("{}\"", link(all))), "invite with actions");
    assert!(
        invite.contains(&again) && invite.contains("Authorise again"),
        "{invite}"
    );
    // Beta has actions off and lacks nothing: no link for it (its owner sees no Alpha either).
    let bea = w.login(BEA).await.unwrap();
    let invite = w.get("/invite", Some(&bea)).await.body;
    assert!(!invite.contains("guild_id="), "{invite}");
    assert!(invite.contains(&link(all)), "actions are on somewhere");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_can_end_the_pause_after_repeated_removals() {
    let w = Web::start(true).await;
    let ada = w.login(ADA).await.unwrap();
    let csrf = w.page_csrf(&ada).await;
    let back = format!("/c/{G}");
    for (guild, says) in [
        (G.to_string(), "joins here again"),
        ("424242".to_owned(), "may not change this"),
    ] {
        let r = w
            .post(
                "/community/resume-joining",
                Some(&ada),
                &[
                    ("csrf", csrf.as_str()),
                    ("guild", guild.as_str()),
                    ("back", back.as_str()),
                ],
            )
            .await;
        let n = r.cookie("pb_notice").expect("a notice");
        let page = w.get(&back, Some(&format!("{ada}; pb_notice={n}"))).await.body;
        assert!(page.contains(says), "{guild}: {page}");
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_pauses_the_whole_bot() {
    let w = Web::start(true).await;
    let g = GuildId(G);
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let system = w.get("/system", Some(&owner)).await.body;
    assert!(system.contains("Pause everywhere"), "{system}");
    let switch = |value: &'static str| {
        [
            ("csrf", csrf.clone()),
            ("scope", "global".to_owned()),
            ("key", "paused".to_owned()),
            ("value", value.to_owned()),
            ("action", "set".to_owned()),
            ("back", "/system".to_owned()),
        ]
    };
    let send = async |cookie: &str, form: [(&'static str, String); 6]| {
        let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.post("/settings", Some(cookie), &f).await
    };
    // An admin cannot.
    let ada = w.login(ADA).await.unwrap();
    let mut by_ada = switch("on");
    by_ada[0].1 = w.page_csrf(&ada).await;
    send(&ada, by_ada).await;
    assert!(!w.engine.settings().current().paused_everywhere());
    // Alpha says "not paused" for itself; the pause everywhere still stops it.
    w.post(
        "/settings",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &format!("server:{G}")),
            ("key", "paused"),
            ("value", "off"),
            ("action", "set"),
            ("back", "/"),
        ],
    )
    .await;
    assert!(w.engine.settings().current().is_tracked(g, UserId(MAX)));
    send(&owner, switch("on")).await;
    let tree = w.engine.settings().current();
    assert!(tree.paused_everywhere() && tree.tracked_for(g).is_empty());
    let page = w.get(&format!("/c/{G}"), Some(&ada)).await.body;
    assert!(page.contains("Paused everywhere"), "{page}");
    assert!(
        !page.contains("Pause here") && !page.contains("Resume here"),
        "the community's own switch is moot"
    );
    assert!(page.contains("The bot is paused everywhere"), "every page says so");
    assert!(
        !page.contains("/system#pause"),
        "only the owner gets the link to switch it back"
    );
    let system = w.get("/system", Some(&owner)).await.body;
    assert!(system.contains("Resume everywhere") && system.contains("/system#pause"));
    let r = send(&owner, switch("off")).await;
    assert!(notice(&w, &owner, &r).await.contains("Resumed in every community"));
    // A community's switch: the next page already shows the other button (the live view follows a moment later).
    let pause_alpha = |value: &'static str| {
        [
            ("csrf", csrf.clone()),
            ("scope", format!("server:{G}")),
            ("key", "paused".to_owned()),
            ("value", value.to_owned()),
            ("action", "set".to_owned()),
            ("back", format!("/c/{G}")),
        ]
    };
    let r = send(&owner, pause_alpha("on")).await;
    assert!(notice(&w, &owner, &r).await.contains("Paused in Alpha"));
    let page = w.get(&format!("/c/{G}"), Some(&owner)).await.body;
    assert!(page.contains("Resume here") && !page.contains("Pause here"), "{page}");
    send(&owner, pause_alpha("off")).await;
    let page = w.get(&format!("/c/{G}"), Some(&owner)).await.body;
    assert!(page.contains("Pause here"), "{page}");
    send(&owner, switch("on")).await;
    send(&owner, switch("off")).await;
    assert!(!w.engine.settings().current().paused_everywhere());
    assert!(w.engine.settings().current().is_tracked(g, UserId(MAX)));
    w.stop().await;
}

/// The notice a response left, as the next page shows it.
async fn notice(w: &Web, cookie: &str, r: &common::Got) -> String {
    let n = r.cookie("pb_notice").expect("a notice");
    let page = w.get("/", Some(&format!("{cookie}; pb_notice={n}"))).await.body;
    let at = page.find("class=\"notice").expect("the notice on the page");
    page[at..].split("</div>").next().unwrap().to_owned()
}

/// Uploads a clip as `cookie`; returns its hash.
async fn upload_clip(w: &Web, cookie: &str, csrf: &str, name: &str) -> String {
    let wav = std::fs::read(pb_testkit::fixture("benign_1.wav")).unwrap();
    let form = reqwest::multipart::Form::new()
        .text("csrf", csrf.to_owned())
        .text("back", "/voice-lines")
        .text("name", name.to_owned())
        .text("lang", "en")
        .part("file", reqwest::multipart::Part::bytes(wav).file_name("clip.wav"));
    let r = w.send(w.http.post(w.url("/clips")).multipart(form), Some(cookie)).await;
    assert_eq!(r.status, 303, "{r:?}");
    let page = w.get("/voice-lines", Some(cookie)).await.body;
    let row = page.find(&format!("value=\"{name}\"")).expect("the clip is listed");
    let at = page[row..].find("/media/clip/").unwrap() + row + 12;
    page[at..].chars().take_while(|c| *c != '"').collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn what_cannot_be_undone_is_confirmed_first() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let back = format!("/c/{G}/reports");
    let confirmed = |form: &[(&'static str, String)]| {
        let mut f = form.to_vec();
        f.push(("confirm", "1".into()));
        f
    };
    let post = async |path: &str, form: &[(&'static str, String)]| {
        let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.post(path, Some(&owner), &f).await
    };

    // Emptying a swear jar.
    let jar = vec![
        ("csrf", csrf.clone()),
        ("guild", G.to_string()),
        ("user", MAX.to_string()),
        ("back", back.clone()),
    ];
    let r = post("/jar/reset", &jar).await;
    let to = r.location.clone().unwrap();
    assert!(to.starts_with("/confirm?what=jar&"), "{to}");
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(page.contains("swear jar?"), "{page}");
    assert!(page.contains("It holds 0 violations in Alpha"), "{page}");
    let r = post("/jar/reset", &confirmed(&jar)).await;
    assert_eq!(r.location.as_deref(), Some(back.as_str()));
    assert!(notice(&w, &owner, &r).await.contains("The swear jar was emptied."));

    // Deleting a recording (this sentence has none: the page says so).
    let sentence = pb_domain::SentenceId::new().to_string();
    let rec = vec![
        ("csrf", csrf.clone()),
        ("sentence", sentence.clone()),
        ("back", back.clone()),
    ];
    let r = post("/evidence/delete", &rec).await;
    let to = r.location.clone().unwrap();
    assert!(
        to.starts_with(&format!("/confirm?what=recording&sentence={sentence}&")),
        "{to}"
    );
    let page = w.get(&to, Some(&owner)).await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("That sentence has no recording."), "{}", page.body);
    let ada = w.login(ADA).await.unwrap();
    assert_eq!(
        w.get(&to, Some(&ada)).await.status,
        404,
        "only the owner deletes recordings"
    );

    // Removing a clip: at once while no voice line uses it …
    let unused = upload_clip(&w, &owner, &csrf, "Unused").await;
    let r = post(
        "/clips/remove",
        &[
            ("csrf", csrf.clone()),
            ("clip", unused.clone()),
            ("back", "/voice-lines".into()),
        ],
    )
    .await;
    assert_eq!(r.location.as_deref(), Some("/voice-lines"));
    assert!(w.engine.clip(&unused.parse().unwrap()).is_none());
    // … and after a confirmation that names the lines using it.
    let used = upload_clip(&w, &owner, &csrf, "Hello there").await;
    w.post(
        "/voice-lines",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &format!("server:{G}")),
            ("line", "greeting"),
            ("op", "add_clip"),
            ("clip", &used),
            ("back", "/"),
        ],
    )
    .await;
    let remove = vec![
        ("csrf", csrf.clone()),
        ("clip", used.clone()),
        ("back", "/voice-lines".into()),
    ];
    let r = post("/clips/remove", &remove).await;
    let to = r.location.clone().unwrap();
    assert!(to.starts_with("/confirm?what=clip&"), "{to}");
    assert!(w.engine.clip(&used.parse().unwrap()).is_some());
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(page.contains("Remove the clip “Hello there”?"), "{page}");
    assert!(page.contains("<li>Greeting (in Alpha)</li>"), "{page}");
    post("/clips/remove", &confirmed(&remove)).await;
    assert!(w.engine.clip(&used.parse().unwrap()).is_none());

    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_forms_fold_the_rare_ones() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    // The settings for every community are pages of their own, one per section, not part of the System page.
    let home = w.get("/settings", Some(&owner)).await.body;
    assert!(home.contains("href=\"/settings/detection\"") && home.contains("href=\"/settings/tracking\""));
    let system = w.get("/system", Some(&owner)).await.body;
    assert!(
        !system.contains("id=\"set-threshold\"") && system.contains("id=\"set-cpu_threads\""),
        "{system}"
    );
    let page = w.get("/settings/detection", Some(&owner)).await.body;
    // The tag of the section's Advanced part.
    let folder = |page: &str| {
        let at = page
            .find("<details id=\"advanced-detection\"")
            .expect("an Advanced part");
        (at, page[at..].split('>').next().unwrap().to_owned())
    };
    let (advanced, tag) = folder(&page);
    assert!(!tag.contains(" open"), "{tag}");
    let threshold = page.find("id=\"set-threshold\"").expect("the threshold");
    let end_silence = page
        .find("id=\"set-end_silence\"")
        .expect("the pause that ends a sentence");
    assert!(
        threshold < advanced && advanced < end_silence,
        "everyday first, the rare ones folded"
    );
    assert!(!page.contains("id=\"set-audience\""), "one section on a page");
    assert!(page.contains("<details class=\"help\">"), "help texts folded");
    assert!(page.contains("<label for=\"in-threshold\">") && page.contains("id=\"in-threshold\""));
    // Saving an advanced setting comes back with it open.
    let open = w.get("/settings/detection?advanced=detection", Some(&owner)).await.body;
    assert!(folder(&open).1.contains(" open"), "opened after saving one");
    assert_eq!(w.get("/settings/nothing", Some(&owner)).await.status, 404);
    assert_eq!(
        w.get("/settings/system", Some(&owner)).await.status,
        404,
        "on the System page"
    );
    // Two settings set here (and the instance, from the start).
    for (key, value) in [("threshold", "0.7"), ("strikes", "2")] {
        w.post(
            "/settings",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("scope", "global"),
                ("key", key),
                ("value", value),
                ("action", "set"),
                ("back", "/settings/detection"),
            ],
        )
        .await;
    }
    let home = w.get("/settings", Some(&owner)).await.body;
    assert!(home.contains("2 changed here"), "{home}");
    // The reset offer counts what it would remove: not the instance (a reset never cuts the bot off from Fluxer).
    assert!(home.contains("2 settings are set here."), "{home}");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_setting_at_a_scope_is_reset_at_once() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let ada = w.login(ADA).await.unwrap();
    let post = async |path: &str, form: &[(&'static str, String)]| {
        let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.post(path, Some(&owner), &f).await
    };
    // A community's settings: the confirmation lists them; an admin's reset leaves the owner's.
    let scope = format!("server:{G}");
    for (key, value) in [("threshold", "0.8"), ("modlog_audio", "on")] {
        post(
            "/settings",
            &[
                ("csrf", csrf.clone()),
                ("scope", scope.clone()),
                ("key", key.into()),
                ("value", value.into()),
                ("action", "set".into()),
                ("back", "/".into()),
            ],
        )
        .await;
    }
    let ca = w.page_csrf(&ada).await;
    let reset = [("csrf", ca.as_str()), ("scope", scope.as_str()), ("back", "/")];
    let r = w.post("/settings/reset", Some(&ada), &reset).await;
    let to = r.location.clone().unwrap();
    assert!(to.starts_with("/confirm?what=reset&scope=server%3A111111"), "{to}");
    let page = w.get(&to, Some(&ada)).await.body;
    assert!(
        page.contains("<li>General threshold</li>") && !page.contains("Audio in the mod log"),
        "{page}"
    );
    let mut confirmed_reset = reset.to_vec();
    confirmed_reset.push(("confirm", "1"));
    let r = w.post("/settings/reset", Some(&ada), &confirmed_reset).await;
    assert!(notice(&w, &ada, &r).await.contains("One setting was reset."));
    let server = pb_domain::Scope::Server { guild: GuildId(G) };
    assert_eq!(setting(&w, server, "threshold"), None);
    assert_eq!(setting(&w, server, "modlog_audio"), Some(serde_json::json!(true)));
    // Bea may not reset Alpha.
    let bea = w.login(BEA).await.unwrap();
    let cb = w.page_csrf(&bea).await;
    let r = w
        .post(
            "/settings/reset",
            Some(&bea),
            &[
                ("csrf", cb.as_str()),
                ("scope", scope.as_str()),
                ("back", "/"),
                ("confirm", "1"),
            ],
        )
        .await;
    assert!(notice(&w, &bea, &r).await.contains("You may not change this."));
    assert_eq!(setting(&w, server, "modlog_audio"), Some(serde_json::json!(true)));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setting_says_where_the_setting_it_needs_lives() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    // Audio in the mod log (global) needs the mod-log channel, which each community chooses.
    let row = |page: &str| {
        let at = page.find("id=\"set-modlog_audio\"").expect("the mod-log audio setting");
        page[at..].split("</form>").next().unwrap().to_owned()
    };
    let system = row(&w.get("/settings/reporting", Some(&owner)).await.body);
    assert!(
        system.contains("Works together with “Mod log channel”, which is set per community:"),
        "{system}"
    );
    // Each community with its channel, linked to that setting there.
    let item = |row: &str, g: u64| {
        let at = row
            .find(&format!("<a href=\"/c/{g}/settings/reporting#set-modlog_channel\">"))
            .unwrap_or_else(|| panic!("a link to {g}'s mod-log channel: {row}"));
        row[at..].split("</li>").next().unwrap().to_owned()
    };
    let alpha = item(&system, G);
    assert!(alpha.contains("Alpha</a>") && alpha.contains("not set"), "{alpha}");
    assert!(item(&system, G2).contains("Beta</a>"));
    w.post(
        "/settings",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &format!("server:{G}")),
            ("key", "modlog_channel"),
            ("value", &LOUNGE.to_string()),
            ("action", "set"),
            ("back", "/"),
        ],
    )
    .await;
    let system = row(&w.get("/settings/reporting", Some(&owner)).await.body);
    let alpha = item(&system, G);
    assert!(alpha.contains("#Lounge"), "{alpha}");
    // In a community both are on the same page: nothing to point at.
    let community = row(&w.get(&format!("/c/{G}/settings/reporting"), Some(&owner)).await.body);
    assert!(!community.contains("Works together"), "{community}");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn say_now_needs_a_call_and_plays_clips() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let clip = upload_clip(&w, &owner, &csrf, "Calm down").await;
    let page = w.get(&format!("/c/{G}/p/{MAX}"), Some(&owner)).await.body;
    assert!(page.contains("<fieldset disabled class=\"say-gate\">"), "{page}");
    assert!(page.contains("They are not in a call."));
    assert!(
        page.contains(&format!("<option value=\"{clip}\">Calm down (en)</option>")),
        "{page}"
    );
    let r = w
        .post(
            "/say",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("guild", &G.to_string()),
                ("user", &MAX.to_string()),
                ("clip", &clip),
                ("back", "/"),
            ],
        )
        .await;
    assert!(notice(&w, &owner, &r).await.contains("They are not in a call."));
    // A voice line can be heard in a chosen language; an unknown one is refused.
    let scope = format!("server:{G}");
    w.post(
        "/voice-lines",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &scope),
            ("line", "greeting"),
            ("op", "add_clip"),
            ("clip", &clip),
            ("back", "/"),
        ],
    )
    .await;
    let lines = w.get(&format!("/c/{G}/voice-lines"), Some(&owner)).await.body;
    assert!(
        lines.contains("As the bot would say it"),
        "a language choice for previews"
    );
    let preview = |lang: &str| format!("/media/preview?scope={scope}&line=greeting&lang={lang}");
    assert_eq!(w.get(&preview("de"), Some(&owner)).await.status, 200);
    assert_eq!(w.get(&preview("no%20language"), Some(&owner)).await.status, 404);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn logging_in_returns_to_the_page_asked_for() {
    let w = Web::start(true).await;
    w.fake.log_in_browser(Some(OWNER));
    let start = w.get(&format!("/login?next=%2Fc%2F{G}%2Fsettings"), None).await;
    let oauth = start.cookie("pb_oauth").unwrap();
    let at_fluxer = w.get(start.location.as_deref().unwrap(), None).await;
    let back = w
        .get(
            at_fluxer.location.as_deref().unwrap(),
            Some(&format!("pb_oauth={oauth}")),
        )
        .await;
    assert_eq!(back.location.as_deref(), Some(format!("/c/{G}/settings").as_str()));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_pages_say_what_to_do_first() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let wall = w.get("/", Some(&owner)).await.body;
    assert!(!wall.contains("first-step"), "Max is tracked: nothing to explain");
    w.engine
        .untrack(GuildId(G), &[UserId(MAX)], pb_store_api::Actor::system())
        .await
        .unwrap();
    // The pages read the live state, which follows the change a moment later.
    let shows = async |path: String, what: &str| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let body = w.get(&path, Some(&owner)).await.body;
            if body.contains(what) {
                return;
            }
            assert!(std::time::Instant::now() < end, "{path} never showed {what:?}: {body}");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    shows("/".into(), "Nobody is tracked yet.").await;
    shows(format!("/c/{G}"), "!pb add @name").await;
    let evidence = w.get(&format!("/c/{G}/p/{MAX}/evidence"), Some(&owner)).await.body;
    assert!(evidence.contains("Settings → Recording"), "{evidence}");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clip_keeps_a_name() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let clip = upload_clip(&w, &owner, &csrf, "Calm down").await;
    let r = w
        .post(
            "/clips/update",
            Some(&owner),
            &[
                ("csrf", &csrf),
                ("clip", &clip),
                ("name", "  "),
                ("back", "/voice-lines"),
            ],
        )
        .await;
    assert!(notice(&w, &owner, &r).await.contains("A clip needs a name."));
    assert_eq!(w.engine.clip(&clip.parse().unwrap()).unwrap().name, "Calm down");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_commands_are_explained_in_the_web_ui() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    // Beside the commands' settings (for every community), with the prefix in use.
    let system = w.get("/settings/commands", Some(&owner)).await.body;
    let at = system
        .find("id=\"chat-commands\"")
        .expect("the chat commands beside their settings");
    assert!(
        system[..at].contains("id=\"section-commands\""),
        "in the chat commands section"
    );
    assert!(
        system.contains("<code>!pb status</code>") && system.contains("no slash commands"),
        "{system}"
    );
    w.post(
        "/settings",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", "global"),
            ("key", "command_prefix"),
            ("value", "?w"),
            ("action", "set"),
            ("back", "/system"),
        ],
    )
    .await;
    // On a community's overview; settings with a command of their own link there.
    let ada = w.login(ADA).await.unwrap();
    let overview = w.get(&format!("/c/{G}"), Some(&ada)).await.body;
    assert!(
        overview.contains("id=\"chat-commands\"") && overview.contains("<code>?w add @user…</code>"),
        "{overview}"
    );
    let settings = w.get(&format!("/c/{G}/settings/reporting"), Some(&ada)).await.body;
    assert!(
        settings.contains(&format!("href=\"/c/{G}#chat-commands\""))
            && settings.contains("<code>?w jar [@user]</code>"),
        "{settings}"
    );
    // A person's settings link only the commands that name a person.
    let person = w.get(&format!("/c/{G}/p/{MAX}/settings"), Some(&ada)).await.body;
    assert!(person.contains("<code>?w set strikes 2 [@user]</code>") && !person.contains("<code>?w pause</code>"));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn id_lists_change_one_entry_at_a_time() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let list = async |cookie: &str, token: &str, scope: &str, key: &str, op: &str, entry: &str, confirm: bool| {
        let mut f = vec![
            ("csrf", token),
            ("scope", scope),
            ("key", key),
            ("op", op),
            ("entry", entry),
            ("back", "/system"),
        ];
        if confirm {
            f.push(("confirm", "1"));
        }
        w.post("/settings/list", Some(cookie), &f).await
    };
    let everywhere = || setting(&w, pb_domain::Scope::Global, "tracked_everywhere");
    // The list is never one text field: entries by name, each removed on its own, and an empty field to add one.
    let row = |page: &str, key: &str| {
        let at = page.find(&format!("id=\"set-{key}\"")).expect("the list setting");
        let rest = &page[at..];
        // Up to the next setting.
        let end = rest[10..].find("id=\"set-").map_or(rest.len(), |e| e + 10);
        rest[..end].to_owned()
    };
    let system = w.get("/settings/tracking", Some(&owner)).await.body;
    let tracked = row(&system, "tracked_everywhere");
    assert!(
        tracked.contains("None yet.") && !tracked.contains("name=\"value\""),
        "{tracked}"
    );
    assert!(tracked.contains("action=\"/settings/list\"") && tracked.contains("name=\"entry\""));
    // Add by ID (one the bot knows by name once she was in a call).
    w.fake.voice_join(G, LOUNGE, ADA);
    let known = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while w.engine.guilds().name(GuildId(G), UserId(ADA)) != "Ada" {
        assert!(std::time::Instant::now() < known, "the bot learns Ada's name");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let r = list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "add",
        &ADA.to_string(),
        false,
    )
    .await;
    assert!(notice(&w, &owner, &r).await.contains("Ada added."));
    assert_eq!(everywhere(), Some(serde_json::json!([ADA.to_string()])));
    // A second time: said so, nothing changes.
    let r = list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "add",
        &format!("<@{ADA}>"),
        false,
    )
    .await;
    assert!(notice(&w, &owner, &r).await.contains("Ada is already on the list."));
    assert_eq!(everywhere(), Some(serde_json::json!([ADA.to_string()])));
    // An ID the bot has never seen is added, with a note; a name nobody has is not.
    let r = list(&owner, &csrf, "global", "tracked_everywhere", "add", "123456789", false).await;
    assert!(notice(&w, &owner, &r).await.contains("has not seen this ID yet"));
    let r = list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "add",
        "Nobody Here",
        false,
    )
    .await;
    assert!(
        notice(&w, &owner, &r)
            .await
            .contains("Nothing called “Nobody Here” is known.")
    );
    assert_eq!(everywhere().and_then(|v| v.as_array().map(Vec::len)), Some(2));
    // Communities by name.
    let r = list(&owner, &csrf, "global", "guild_allowlist", "add", "beta", false).await;
    assert!(notice(&w, &owner, &r).await.contains("Beta added."));
    assert_eq!(
        setting(&w, pb_domain::Scope::Global, "guild_allowlist"),
        Some(serde_json::json!([G2.to_string()]))
    );
    // Removing asks first, names the consequence (here: the last allowed community), then removes that entry only.
    let r = list(
        &owner,
        &csrf,
        "global",
        "guild_allowlist",
        "remove",
        &G2.to_string(),
        false,
    )
    .await;
    let to = r.location.clone().unwrap();
    assert!(
        to.starts_with("/confirm?what=list-remove&scope=global&key=guild_allowlist&entry=777777"),
        "{to}"
    );
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(page.contains("Remove Beta from “Only these communities”?"), "{page}");
    assert!(page.contains("the last community on the list"), "{page}");
    let r = list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "remove",
        "123456789",
        true,
    )
    .await;
    assert!(notice(&w, &owner, &r).await.contains("123456789 removed."));
    assert_eq!(everywhere(), Some(serde_json::json!([ADA.to_string()])));
    let r = list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "remove",
        "123456789",
        true,
    )
    .await;
    assert!(notice(&w, &owner, &r).await.contains("is not on the list."));
    // A community's admin may not change the owner's lists; roles are added by name.
    let bea = w.login(BEA).await.unwrap();
    let cb = w.page_csrf(&bea).await;
    let r = list(
        &bea,
        &cb,
        "global",
        "tracked_everywhere",
        "remove",
        &ADA.to_string(),
        true,
    )
    .await;
    assert!(notice(&w, &bea, &r).await.contains("You may not change this."));
    assert_eq!(everywhere(), Some(serde_json::json!([ADA.to_string()])));
    let r = list(
        &owner,
        &csrf,
        &format!("server:{G}"),
        "admin_role_ids",
        "add",
        "Mods",
        false,
    )
    .await;
    assert!(notice(&w, &owner, &r).await.contains("Mods added."));
    let server = pb_domain::Scope::Server { guild: GuildId(G) };
    assert_eq!(
        setting(&w, server, "admin_role_ids"),
        Some(serde_json::json!(["888888"]))
    );
    // On a person's page the owner tracks them in every community, or stops it.
    let person = w.get(&format!("/c/{G}/p/{MAX}"), Some(&owner)).await.body;
    assert!(person.contains("Track in every community"), "{person}");
    list(
        &owner,
        &csrf,
        "global",
        "tracked_everywhere",
        "add",
        &MAX.to_string(),
        false,
    )
    .await;
    let person = w.get(&format!("/c/{G}/p/{MAX}"), Some(&owner)).await.body;
    assert!(person.contains("Stop tracking in every community"));
    // The community's tracked list says where people tracked everywhere are taken off (the live state follows).
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let page = w.get(&format!("/c/{G}"), Some(&owner)).await.body;
        if page.contains("on their own page or on the System page") {
            break;
        }
        assert!(std::time::Instant::now() < end, "{page}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_community_page_opened_later_shows_earlier_violations() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let rec = pb_store_api::SentenceRecord {
        id: pb_domain::SentenceId::new(),
        guild: GuildId(G),
        channel: pb_domain::ChannelId(LOUNGE),
        user: UserId(MAX),
        started: jiff::Timestamp::now(),
        dur_ms: 1500,
        level_db: None,
        cut: pb_store_api::CutCause::Pause,
        scores: [0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.9, 0.0],
        language: pb_domain::ClfLang::En,
        thresholds: vec![(pb_domain::Label::Profanity, 0.5)],
        flagged: vec![pb_domain::Label::Profanity],
        decision: pb_store_api::DecisionRecord::Warn {
            label: pb_domain::Label::Profanity,
            score: 0.9,
            step: 1,
            count: 1,
        },
        jar: true,
        audio: None,
        infer_ms: None,
        cut_to_verdict_ms: None,
        model: "test".into(),
        source: pb_store_api::SentenceSource::Live,
    };
    assert!(
        w.engine
            .record(vec![pb_store_api::Event::Sentence(Box::new(rec))])
            .await
    );
    let shows = async |path: &str| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let body = w.get(path, Some(&owner)).await.body;
            if body.contains("Warned (step 1)") {
                return;
            }
            assert!(
                std::time::Instant::now() < end,
                "{path} never showed the violation: {body}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    // Once the index has it (the Reports page reads the index) …
    shows("/reports").await;
    // … the community's live list, made when the page is first opened, starts with it.
    shows(&format!("/c/{G}")).await;
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_page_has_an_element_id_twice() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    for path in [
        "/".to_owned(),
        "/system".to_owned(),
        "/settings".to_owned(),
        "/settings/detection".to_owned(),
        "/settings/tracking".to_owned(),
        "/voice-lines".to_owned(),
        "/invite".to_owned(),
        format!("/c/{G}"),
        format!("/c/{G}/settings"),
        format!("/c/{G}/settings/detection"),
        format!("/c/{G}/voice-lines"),
        format!("/c/{G}/p/{MAX}"),
        format!("/c/{G}/p/{MAX}/settings"),
        format!("/c/{G}/p/{MAX}/voice-lines"),
    ] {
        let body = w.get(&path, Some(&owner)).await.body;
        let mut seen = std::collections::BTreeSet::new();
        for part in body.split(" id=\"").skip(1) {
            let id = part.split('"').next().unwrap();
            assert!(seen.insert(id.to_owned()), "{path}: id {id:?} twice");
        }
    }
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_voice_line_says_which_line_the_bot_uses() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let lines = format!("/c/{G}/voice-lines");
    let row = |page: &str, line: &str| {
        let at = page.find(&format!("id=\"line-{line}\"")).expect("the line");
        page[at..]
            .split("class=\"vline-head\"")
            .nth(1)
            .unwrap()
            .split("</div>")
            .next()
            .unwrap()
            .to_owned()
    };
    let page = w.get(&lines, Some(&owner)).await.body;
    assert!(
        row(&page, "warning.profanity.any").contains("built in"),
        "nothing set: the built-in texts"
    );
    // A line for any type at Alpha is what a profanity warning there says.
    w.post(
        "/voice-lines",
        Some(&owner),
        &[
            ("csrf", &csrf),
            ("scope", &format!("server:{G}")),
            ("line", "warning.any.any"),
            ("op", "set_text"),
            ("lang", "en"),
            ("text", "{name}, language!"),
            ("back", "/"),
        ],
    )
    .await;
    let page = w.get(&lines, Some(&owner)).await.body;
    assert!(
        row(&page, "warning.any.any").contains("Own line"),
        "{}",
        row(&page, "warning.any.any")
    );
    let profanity = row(&page, "warning.profanity.any");
    assert!(
        profanity.contains("uses “Warning · any type · every step” (this community)"),
        "{profanity}"
    );
    // Max's page: Alpha's line, by the same rule.
    let person = w.get(&format!("/c/{G}/p/{MAX}/voice-lines"), Some(&owner)).await.body;
    assert!(row(&person, "warning.any.any").contains("from this community"));
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_or_refused_setting_says_so_next_to_it() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let back = format!("/c/{G}/settings/detection");
    let scope = format!("server:{G}");
    let save = async |value: &str| {
        w.post(
            "/settings",
            Some(&owner),
            &[
                ("csrf", csrf.as_str()),
                ("scope", scope.as_str()),
                ("key", "strikes"),
                ("value", value),
                ("action", "set"),
                ("back", back.as_str()),
            ],
        )
        .await
    };
    let after = async |r: &common::Got| {
        let n = r.cookie("pb_notice").expect("a notice");
        let page = w
            .get(r.location.as_deref().unwrap(), Some(&format!("{owner}; pb_notice={n}")))
            .await
            .body;
        let at = page.find("id=\"set-strikes\"").expect("the setting");
        let end = page[at + 10..].find("id=\"set-").map_or(page.len(), |e| at + 10 + e);
        (page[at..end].to_owned(), page)
    };
    // Refused: the reason under the field, and what was typed stays in it.
    let r = save("lots").await;
    let (row, _) = after(&r).await;
    assert!(row.contains("class=\"field-note error\""), "{row}");
    assert!(row.contains("value=\"lots\""), "what was typed stays: {row}");
    // Saved: the top says which setting, the setting says so too.
    let r = save("3").await;
    let (row, page) = after(&r).await;
    assert!(page.contains("Saved: Strikes before a warning."), "{page}");
    assert!(
        row.contains("class=\"field-note ok\"") && row.contains("value=\"3\""),
        "{row}"
    );
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resetting_a_line_the_connection_or_logging_out_everywhere_asks_first() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let scope = format!("server:{G}");
    // A voice line's own texts and clips.
    let line = |op: &'static str, confirm: bool| {
        let mut f = vec![
            ("csrf", csrf.clone()),
            ("scope", scope.clone()),
            ("line", "greeting".to_owned()),
            ("op", op.to_owned()),
            ("back", format!("/c/{G}/voice-lines")),
        ];
        if op == "set_text" {
            f.extend([("lang", "en".to_owned()), ("text", "Hello {name}".to_owned())]);
        }
        if confirm {
            f.push(("confirm", "1".to_owned()));
        }
        f
    };
    let post = async |path: &str, form: Vec<(&'static str, String)>| {
        let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.post(path, Some(&owner), &f).await
    };
    post("/voice-lines", line("set_text", false)).await;
    let greeting = || {
        w.engine
            .settings()
            .current()
            .voice_lines(pb_domain::Scope::Server { guild: GuildId(G) })
            .and_then(|s| s.get(&"greeting".parse().unwrap()).cloned())
    };
    assert!(greeting().is_some());
    let r = post("/voice-lines", line("clear", false)).await;
    let to = r.location.clone().unwrap();
    assert!(to.starts_with("/confirm?what=line-clear&"), "{to}");
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(page.contains("Remove the own clips and texts of “Greeting”?"), "{page}");
    assert!(
        page.contains("The line loses its 0 clips and its text in Alpha."),
        "{page}"
    );
    assert!(greeting().is_some());
    post("/voice-lines", line("clear", true)).await;
    assert!(greeting().is_none());
    // The Fluxer instance: what it goes back to, and that the bot reconnects.
    let r = post(
        "/settings",
        vec![
            ("csrf", csrf.clone()),
            ("scope", "global".to_owned()),
            ("key", "instance".to_owned()),
            ("action", "clear".to_owned()),
            ("back", "/system".to_owned()),
        ],
    )
    .await;
    let to = r.location.clone().unwrap();
    assert!(
        to.starts_with("/confirm?what=setting-clear&scope=global&key=instance"),
        "{to}"
    );
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(
        page.contains("https://api.fluxer.app") && page.contains("connects to Fluxer again"),
        "{page}"
    );
    assert!(
        setting(&w, pb_domain::Scope::Global, "instance").is_some(),
        "unchanged until confirmed"
    );
    // Logging out on all devices.
    let r = w
        .post(
            "/auth/logout",
            Some(&owner),
            &[("csrf", &csrf), ("everywhere", "1"), ("back", "/system")],
        )
        .await;
    let to = r.location.clone().unwrap();
    assert_eq!(to, "/confirm?what=logout-all&back=%2Fsystem");
    assert!(
        w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""),
        "still logged in"
    );
    let page = w.get(&to, Some(&owner)).await.body;
    assert!(page.contains("Log out on all devices?"), "{page}");
    w.post(
        "/auth/logout",
        Some(&owner),
        &[("csrf", &csrf), ("everywhere", "1"), ("confirm", "1")],
    )
    .await;
    assert!(
        !w.get("/", Some(&owner)).await.body.contains("class=\"sidebar\""),
        "logged out"
    );
    w.stop().await;
}

/// The value a page's input for `key` shows (`value.<key>`).
fn shown_value(page: &str, key: &str) -> String {
    let at = page.find(&format!("name=\"value.{key}\"")).expect("the input");
    let tag = page[at..].split('>').next().unwrap();
    tag.split("value=\"")
        .nth(1)
        .map(|v| v.split('"').next().unwrap().to_owned())
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_section_form_saves_what_was_changed_and_only_that() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let path = format!("/c/{G}/settings/detection");
    let scope = format!("server:{G}");
    let server = pb_domain::Scope::Server { guild: GuildId(G) };
    let page = w.get(&path, Some(&owner)).await.body;
    // One form for the section and one "Save changes"; inputs are named after their setting.
    assert_eq!(page.matches("class=\"section-form\"").count(), 1, "{page}");
    assert!(page.contains("name=\"value.strikes\"") && page.contains("Save changes"));
    let (threshold, end_silence) = (shown_value(&page, "threshold"), shown_value(&page, "end_silence"));
    let save = async |cookie: &str, token: &str, values: &[(&str, &str)], extra: &[(&str, &str)]| {
        let keys: Vec<&str> = values.iter().map(|(k, _)| *k).collect();
        let keys = keys.join(",");
        let mut form: Vec<(String, String)> = vec![
            ("csrf".into(), token.into()),
            ("scope".into(), scope.clone()),
            ("back".into(), path.clone()),
            ("keys".into(), keys),
            ("action".into(), "save".into()),
        ];
        form.extend(values.iter().map(|(k, v)| (format!("value.{k}"), (*v).to_owned())));
        form.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        w.post("/settings", Some(cookie), &f).await
    };
    // Two fields changed in one save: both are kept; the fields shown as they were are not stored (they keep
    // following the settings for every community).
    let r = save(
        &owner,
        &csrf,
        &[
            ("strikes", "3"),
            ("strike_window", "30s"),
            ("threshold", &threshold),
            ("end_silence", &end_silence),
        ],
        &[],
    )
    .await;
    assert!(
        notice(&w, &owner, &r)
            .await
            .contains("Saved: Strikes before a warning, Strike window."),
        "{r:?}"
    );
    assert_eq!(setting(&w, server, "strikes"), Some(serde_json::json!(3)));
    assert!(setting(&w, server, "strike_window").is_some());
    assert_eq!(setting(&w, server, "threshold"), None, "not pinned");
    assert_eq!(setting(&w, server, "end_silence"), None, "not pinned");
    // One refused, one saved: the refused one is said next to it (as typed); the page comes back there, with the
    // section's Advanced part (the strike window is in it) open.
    let r = save(&owner, &csrf, &[("strikes", "lots"), ("strike_window", "45s")], &[]).await;
    assert_eq!(
        r.location.as_deref(),
        Some(format!("{path}?advanced=detection#set-strikes").as_str())
    );
    let text = notice(&w, &owner, &r).await;
    assert!(
        text.contains("Saved: Strike window.") && text.contains("Not saved: Strikes before a warning (see below)."),
        "{text}"
    );
    assert_eq!(setting(&w, server, "strikes"), Some(serde_json::json!(3)));
    // "Use inherited" on one setting saves the rest of the form too.
    save(
        &owner,
        &csrf,
        &[("strikes", "3"), ("threshold", "0.8")],
        &[("clear", "strikes")],
    )
    .await;
    assert_eq!(setting(&w, server, "strikes"), None);
    assert_eq!(setting(&w, server, "threshold"), Some(serde_json::json!(0.8)));
    // An admin's form never changes what only the owner may change, even when sent.
    let ada = w.login(ADA).await.unwrap();
    let ca = w.page_csrf(&ada).await;
    let reporting = format!("/c/{G}/settings/reporting");
    let page = w.get(&reporting, Some(&ada)).await.body;
    let keys_at = page.find("name=\"keys\" value=\"").unwrap() + 19;
    let keys = page[keys_at..].split('"').next().unwrap();
    assert!(!keys.split(',').any(|k| k == "modlog_audio"), "{keys}");
    save(&ada, &ca, &[("modlog_audio", "on")], &[]).await;
    assert_eq!(setting(&w, server, "modlog_audio"), None);
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sections_are_grouped_and_say_what_does_nothing_now() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    // The greeting is a warning-section switch now; there is no greeting section.
    let warnings = w.get(&format!("/c/{G}/settings/warning"), Some(&owner)).await.body;
    assert!(
        warnings.contains("id=\"set-greet_enabled\"") && warnings.contains("Warnings"),
        "{warnings}"
    );
    assert_eq!(
        w.get(&format!("/c/{G}/settings/greeting"), Some(&owner)).await.status,
        404
    );
    let home = w.get("/settings", Some(&owner)).await.body;
    assert!(home.contains("Mod log and reports") && home.contains("Admins and chat commands"));
    // With one strike the strike window does nothing: said under it.
    let detection = w.get(&format!("/c/{G}/settings/detection"), Some(&owner)).await.body;
    let at = detection.find("id=\"set-strike_window\"").unwrap();
    let row = &detection[at..at + detection[at + 10..].find("id=\"set-").unwrap() + 10];
    assert!(row.contains("Only used with more than one strike"), "{row}");
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_setting_says_where_its_value_comes_from() {
    let w = Web::start(true).await;
    let owner = w.login(OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    for (scope, value) in [("global".to_owned(), "0.7"), (format!("server:{G}"), "0.4")] {
        w.post(
            "/settings",
            Some(&owner),
            &[
                ("csrf", csrf.as_str()),
                ("scope", scope.as_str()),
                ("key", "threshold"),
                ("value", value),
                ("action", "set"),
                ("back", "/"),
            ],
        )
        .await;
    }
    let row = |page: &str, key: &str| {
        let at = page.find(&format!("id=\"set-{key}\"")).expect("the setting");
        let end = page[at + 10..].find("id=\"set-").map_or(page.len(), |e| at + 10 + e);
        page[at..end].to_owned()
    };
    // For every community: changed, with the default to go back to, and the community that has its own value.
    let global = row(&w.get("/settings/detection", Some(&owner)).await.body, "threshold");
    assert!(
        global.contains("Changed · default: 0.5") && global.contains("Back to the default"),
        "{global}"
    );
    assert!(
        global.contains("Changed in one community:")
            && global.contains(&format!("href=\"/c/{G}/settings/detection#set-threshold\">Alpha</a>")),
        "{global}"
    );
    // In Alpha: its own value, and what every community has.
    let alpha = w.get(&format!("/c/{G}/settings/detection"), Some(&owner)).await.body;
    let threshold = row(&alpha, "threshold");
    assert!(
        threshold.contains("Changed for Alpha · every community: 0.7")
            && threshold.contains("Use the value for every community"),
        "{threshold}"
    );
    assert!(row(&alpha, "strikes").contains("Same as for every community"));
    // For a person: the same as in their community.
    let person = w.get(&format!("/c/{G}/p/{MAX}/settings"), Some(&owner)).await.body;
    assert!(row(&person, "threshold").contains("Same as in Alpha"));
    // The community's summary lists what is changed there, with the value.
    let home = w.get(&format!("/c/{G}/settings"), Some(&owner)).await.body;
    assert!(home.contains("General threshold: <b>0.4</b>"), "{home}");
    w.stop().await;
}
