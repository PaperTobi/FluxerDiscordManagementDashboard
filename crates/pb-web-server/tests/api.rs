//! The read API at /api/v1: tokens, scopes, communities, leaderboards, violations, the OpenAPI document.
#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use common::{G, G2, MAX, Web};
use pb_api_proto::v1::{Leaderboard, Scope, Source, Violations};
use pb_domain::{ChannelId, GuildId, Label, MessageId, SentenceId, UserId};
use pb_store_api::{ChatRecord, DecisionRecord, Event};

async fn get(w: &Web, path: &str, token: Option<&str>) -> (u16, serde_json::Value) {
    let mut req = w.http.get(w.url(&format!("/api/v1{path}")));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(serde_json::Value::Null))
}

fn warn(count: u32) -> DecisionRecord {
    DecisionRecord::Warn {
        label: Label::Profanity,
        score: 1.0,
        step: 1,
        count,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_read_what_their_scopes_and_communities_allow() {
    let w = Web::start(true).await;
    // Max said something twice in Alpha's chat (both into the swear jar).
    let chat = |n: u64, count: u32| ChatRecord {
        id: SentenceId::new(),
        guild: GuildId(G),
        channel: ChannelId(5),
        message: MessageId(n),
        user: UserId(MAX),
        at: jiff::Timestamp::now(),
        text: format!("listed word {n}"),
        matches: vec!["word".into()],
        decision: warn(count),
        jar: true,
    };
    let r = w
        .log
        .append(vec![
            Event::ChatFlagged(Box::new(chat(1, 1))).to_new(None).unwrap(),
            Event::ChatFlagged(Box::new(chat(2, 2))).to_new(None).unwrap(),
        ])
        .await
        .unwrap();
    pb_store_api::Index::caught_up(&*w.index, r[1].seq).await;

    // Without a token, or with one that does not exist: 401.
    assert_eq!(get(&w, "/communities", None).await.0, 401);
    assert_eq!(get(&w, "/communities", Some("pbk_nope")).await.0, 401);
    let (status, root) = get(&w, "/", None).await;
    assert_eq!((status, root["version"].as_str()), (200, Some("v1")));

    // A leaderboard token sees every community and the jar, not the violations.
    let (_, board) = w
        .api
        .create_token("board".into(), &[Scope::Leaderboard], Vec::new(), None)
        .await
        .unwrap();
    let (status, list) = get(&w, "/communities", Some(&board)).await;
    assert_eq!(status, 200);
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(names, ["Alpha", "Beta"]);
    let (status, lb) = get(&w, &format!("/communities/{G}/leaderboard"), Some(&board)).await;
    assert_eq!(status, 200);
    let lb: Leaderboard = serde_json::from_value(lb).unwrap();
    assert_eq!(lb.community.name, "Alpha");
    assert_eq!(
        (lb.places[0].rank, lb.places[0].count, lb.places[0].person.id.as_str()),
        (1, 2, &*MAX.to_string())
    );
    assert_eq!(
        get(&w, &format!("/communities/{G}/violations"), Some(&board)).await.0,
        403
    );
    assert_eq!(get(&w, "/communities/999/leaderboard", Some(&board)).await.0, 404);

    // A token for Beta only does not see Alpha (as if it did not exist).
    let (_, beta) = w
        .api
        .create_token("beta".into(), &[Scope::Leaderboard], vec![GuildId(G2)], None)
        .await
        .unwrap();
    assert_eq!(
        get(&w, &format!("/communities/{G}/leaderboard"), Some(&beta)).await.0,
        404
    );
    assert_eq!(
        get(&w, "/communities", Some(&beta)).await.1.as_array().unwrap().len(),
        1
    );

    // Violations newest first, a page at a time; the text only with the details scope.
    let (made, violations) = w
        .api
        .create_token("violations".into(), &[Scope::Violations], Vec::new(), None)
        .await
        .unwrap();
    let (status, page) = get(&w, &format!("/communities/{G}/violations?limit=1"), Some(&violations)).await;
    assert_eq!(status, 200);
    let page: Violations = serde_json::from_value(page).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!((page.items[0].source, page.items[0].count), (Source::Chat, 2));
    assert_eq!(page.items[0].text, None);
    let next = page.next.expect("a next page");
    let (_, older) = get(
        &w,
        &format!("/communities/{G}/violations?limit=1&cursor={next}"),
        Some(&violations),
    )
    .await;
    let older: Violations = serde_json::from_value(older).unwrap();
    assert_eq!(older.items[0].count, 1);
    let (_, details) = w
        .api
        .create_token("details".into(), &[Scope::Violations, Scope::Details], Vec::new(), None)
        .await
        .unwrap();
    let (_, page) = get(&w, &format!("/communities/{G}/violations"), Some(&details)).await;
    let page: Violations = serde_json::from_value(page).unwrap();
    assert_eq!(page.items[0].text.as_deref(), Some("listed word 2"));

    // A revoked token stops working at once; the file keeps only hashes.
    assert!(w.api.revoke_token(&made.id).await.unwrap());
    assert_eq!(
        get(&w, &format!("/communities/{G}/violations"), Some(&violations))
            .await
            .0,
        401
    );
    let file = std::fs::read_to_string(w.dir.path().join("api.json")).unwrap();
    assert!(!file.contains(&board) && file.contains("\"hash\""));

    // The OpenAPI document describes the routes and the token.
    let (status, doc) = get(&w, "/openapi.json", None).await;
    assert_eq!(status, 200);
    assert!(doc["openapi"].as_str().unwrap().starts_with("3.1"));
    assert!(
        doc["paths"]["/communities/{community}/leaderboard"].is_object(),
        "{doc}"
    );
    assert!(doc["components"]["securitySchemes"]["token"].is_object());
    w.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_makes_and_ends_tokens_on_the_api_page() {
    let w = Web::start(true).await;
    let owner = w.login(common::OWNER).await.unwrap();
    let csrf = w.page_csrf(&owner).await;
    let made = w
        .post(
            "/system/api/create",
            Some(&owner),
            &[
                ("csrf", csrf.as_str()),
                ("back", "/system/api"),
                ("name", "Website"),
                ("scope", "leaderboard"),
                ("community", &G.to_string()),
            ],
        )
        .await;
    let notice = made.cookie("pb_notice").expect("told");
    let page = w
        .get("/system/api", Some(&format!("{owner}; pb_notice={notice}")))
        .await
        .body;
    let at = page.find("pbk_").expect("the token, once");
    let token: String = page[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    assert_eq!(
        get(&w, &format!("/communities/{G}/leaderboard"), Some(&token)).await.0,
        200
    );
    assert_eq!(
        get(&w, &format!("/communities/{G2}/leaderboard"), Some(&token)).await.0,
        404
    );
    // Listed (used just now), never shown again; ended from the same page.
    let page = w.get("/system/api", Some(&owner)).await.body;
    assert!(page.contains("Website") && !page.contains("pbk_"), "{page}");
    let id = w.api.tokens()[0].id.clone();
    assert!(w.api.tokens()[0].last_used.is_some());
    w.post(
        "/system/api/revoke",
        Some(&owner),
        &[("csrf", csrf.as_str()), ("back", "/system/api"), ("id", id.as_str())],
    )
    .await;
    assert_eq!(
        get(&w, &format!("/communities/{G}/leaderboard"), Some(&token)).await.0,
        401
    );
    // Admins who are not the owner do not get the page.
    let ada = w.login(common::ADA).await.unwrap();
    assert_eq!(w.get("/system/api", Some(&ada)).await.status, 404);
    w.stop().await;
}
