//! Version 1. The bot owner makes tokens (System → API); a request names one with `Authorization: Bearer pbk_…`. A
//! token reads what its scopes allow, in its communities (all when it names none).

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pb_api_proto::v1::{self as proto, ApiError, Scope};
use pb_domain::{GuildId, UserId};
use pb_engine::Engine;
use pb_store_api::{
    ApiConfig, ApiFile, ApiToken, ChatRow, Cursor, DecisionRecord, Index, SentenceFilter, SentenceKind, SentenceRow,
    StoreError,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use utoipa::OpenApi;

/// The API's state: the engine and index it reads, and its tokens.
pub struct Api {
    engine: Arc<Engine>,
    index: Arc<dyn Index>,
    file: Arc<dyn ApiFile>,
    config: RwLock<ApiConfig>,
    write: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    // The system's randomness; there is nothing sensible to do without it.
    if getrandom::fill(&mut b).is_err() {
        tracing::error!("the system gave no randomness for an API token");
    }
    hex(&b)
}

/// Who called, and what they may read.
#[derive(Debug, Clone)]
struct Caller {
    scopes: BTreeSet<Scope>,
    communities: Vec<GuildId>,
}

impl Caller {
    fn may(&self, scope: Scope) -> Result<(), Answer> {
        if self.scopes.contains(&scope) {
            Ok(())
        } else {
            Err(Answer::error(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("this token may not read {}", scope.as_str()),
            ))
        }
    }

    fn sees(&self, g: GuildId) -> bool {
        self.communities.is_empty() || self.communities.contains(&g)
    }
}

/// An answer that is not the one asked for.
#[derive(Debug)]
struct Answer(StatusCode, ApiError);

impl Answer {
    fn error(status: StatusCode, error: &str, message: impl Into<String>) -> Answer {
        Answer(
            status,
            ApiError {
                error: error.to_owned(),
                message: message.into(),
            },
        )
    }

    fn store(e: &StoreError) -> Answer {
        Answer::error(StatusCode::SERVICE_UNAVAILABLE, "unavailable", e.to_string())
    }
}

impl IntoResponse for Answer {
    fn into_response(self) -> Response {
        let mut r = (self.0, Json(self.1)).into_response();
        if self.0 == StatusCode::UNAUTHORIZED {
            r.headers_mut()
                .insert(header::WWW_AUTHENTICATE, http::HeaderValue::from_static("Bearer"));
        }
        r
    }
}

impl Api {
    pub async fn load(engine: Arc<Engine>, index: Arc<dyn Index>, file: Arc<dyn ApiFile>) -> Result<Arc<Api>, StoreError> {
        let config = file.load().await?;
        Ok(Arc::new(Api {
            engine,
            index,
            file,
            config: RwLock::new(config),
            write: tokio::sync::Mutex::new(()),
        }))
    }

    fn config(&self) -> std::sync::RwLockReadGuard<'_, ApiConfig> {
        self.config.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn change(&self, f: impl FnOnce(&mut ApiConfig)) -> Result<(), StoreError> {
        let _w = self.write.lock().await;
        let mut next = self.config().clone();
        f(&mut next);
        self.file.save(&next).await?;
        *self.config.write().unwrap_or_else(std::sync::PoisonError::into_inner) = next;
        Ok(())
    }

    /// The tokens (their hashes stay inside).
    pub fn tokens(&self) -> Vec<ApiToken> {
        self.config().tokens.clone()
    }

    /// Makes a token; the token itself is returned once and only its hash is kept.
    pub async fn create_token(
        &self,
        name: String,
        scopes: &[Scope],
        communities: Vec<GuildId>,
        by: Option<UserId>,
    ) -> Result<(ApiToken, String), StoreError> {
        let secret = format!("pbk_{}", random_hex(32));
        let token = ApiToken {
            id: random_hex(4),
            name: name.trim().to_owned(),
            hash: hash(&secret),
            scopes: scopes.iter().map(|s| s.as_str().to_owned()).collect(),
            communities,
            created: jiff::Timestamp::now(),
            by,
            last_used: None,
        };
        let t = token.clone();
        self.change(|c| c.tokens.push(t)).await?;
        Ok((token, secret))
    }

    /// Ends a token; `false` when there was none with that id.
    pub async fn revoke_token(&self, id: &str) -> Result<bool, StoreError> {
        let had = self.config().tokens.iter().any(|t| t.id == id);
        if had {
            self.change(|c| c.tokens.retain(|t| t.id != id)).await?;
        }
        Ok(had)
    }

    fn caller(&self, headers: &HeaderMap) -> Result<Caller, Answer> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(|| Answer::error(StatusCode::UNAUTHORIZED, "unauthorized", "send Authorization: Bearer <token>"))?;
        let h = hash(token);
        let config = self.config();
        let t = config
            .tokens
            .iter()
            .find(|t| t.hash == h)
            .ok_or_else(|| Answer::error(StatusCode::UNAUTHORIZED, "unauthorized", "unknown token"))?;
        Ok(Caller {
            scopes: t.scopes.iter().filter_map(|s| Scope::parse(s)).collect(),
            communities: t.communities.clone(),
        })
    }

    /// The routes, to be nested at `/api/v1`.
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/", get(root))
            .route("/openapi.json", get(openapi))
            .route("/communities", get(communities))
            .route("/communities/{g}/leaderboard", get(leaderboard))
            .route("/communities/{g}/violations", get(violations))
            .with_state(self.clone())
    }

    fn community(&self, g: GuildId) -> proto::Community {
        proto::Community {
            id: g.to_string(),
            name: self.engine.guilds().guild_name(g),
            icon_url: self.engine.icon_url(g),
        }
    }

    fn person(&self, g: GuildId, u: UserId) -> proto::Person {
        let who = self.engine.who(g, u);
        proto::Person {
            id: u.to_string(),
            name: who.name,
            avatar_url: who.avatar,
        }
    }

    /// A community the caller may see (else not found, as if it did not exist).
    fn visible(&self, caller: &Caller, g: &str) -> Result<GuildId, Answer> {
        g.parse::<GuildId>()
            .ok()
            .filter(|g| caller.sees(*g) && self.engine.guilds().available().contains(g))
            .ok_or_else(|| Answer::error(StatusCode::NOT_FOUND, "not_found", "no such community"))
    }
}

/// The API's description.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Profanity Watch API",
        version = "1",
        description = "Read what the bot knows: communities, swear-jar leaderboards and violations. Every route needs a token (Authorization: Bearer pbk_…) that the bot owner makes on the System page; a token reads what its scopes allow, in its communities."
    ),
    servers((url = "/api/v1")),
    paths(root, communities, leaderboard, violations),
    components(schemas(
        proto::Community, proto::Person, proto::Place, proto::Leaderboard, proto::Violation, proto::Violations,
        proto::Source, proto::ApiError, proto::Scope, proto::Delivery, proto::EventKind, proto::DayTotal
    )),
    modifiers(&BearerAuth)
)]
pub struct ApiDoc;

struct BearerAuth;

impl utoipa::Modify for BearerAuth {
    fn modify(&self, doc: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        let components = doc.components.get_or_insert_with(Default::default);
        components.add_security_scheme("token", SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)));
        doc.security = Some(vec![utoipa::openapi::security::SecurityRequirement::new(
            "token",
            Vec::<String>::new(),
        )]);
    }
}

/// The API's version and where its description is.
#[utoipa::path(get, path = "/", responses((status = 200, description = "The version")))]
async fn root() -> Json<serde_json::Value> {
    Json(serde_json::json!({"version": "v1", "openapi": format!("{}/openapi.json", proto::BASE)}))
}

/// The OpenAPI 3.1 document of this API.
async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// The communities the token may read, by name.
#[utoipa::path(get, path = "/communities", responses(
    (status = 200, body = Vec<proto::Community>),
    (status = 401, body = ApiError),
))]
async fn communities(State(api): State<Arc<Api>>, headers: HeaderMap) -> Result<Json<Vec<proto::Community>>, Answer> {
    let caller = api.caller(&headers)?;
    if !Scope::ALL.iter().any(|s| caller.scopes.contains(s)) {
        return Err(Answer::error(StatusCode::FORBIDDEN, "forbidden", "this token may read nothing"));
    }
    let mut out: Vec<proto::Community> = api
        .engine
        .guilds()
        .available()
        .into_iter()
        .filter(|g| caller.sees(*g))
        .map(|g| api.community(g))
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Json(out))
}

/// A community's swear jar, most first (people with the same count share a place).
#[utoipa::path(get, path = "/communities/{community}/leaderboard",
    params(("community" = String, Path, description = "The community's id")),
    responses((status = 200, body = proto::Leaderboard), (status = 403, body = ApiError), (status = 404, body = ApiError)))]
async fn leaderboard(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Path(g): Path<String>,
) -> Result<Json<proto::Leaderboard>, Answer> {
    let caller = api.caller(&headers)?;
    caller.may(Scope::Leaderboard)?;
    let g = api.visible(&caller, &g)?;
    let mut rows = api.index.jar(Some(g)).await.map_err(|e| Answer::store(&e))?;
    rows.retain(|r| r.count > 0);
    rows.sort_by(|a, b| b.count.cmp(&a.count));
    let mut places = Vec::with_capacity(rows.len());
    let mut rank = 0u32;
    let mut last = None;
    for (i, r) in rows.iter().enumerate() {
        if last != Some(r.count) {
            rank = u32::try_from(i).unwrap_or(u32::MAX).saturating_add(1);
            last = Some(r.count);
        }
        places.push(proto::Place {
            rank,
            person: api.person(g, r.user),
            count: r.count,
        });
    }
    Ok(Json(proto::Leaderboard {
        community: api.community(g),
        places,
    }))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct PageQuery {
    /// The `next` of the page before.
    cursor: Option<String>,
    /// How many (default 50).
    limit: Option<u32>,
}

fn decision(d: &DecisionRecord) -> &'static str {
    match d {
        DecisionRecord::Warn { .. } => "warn",
        DecisionRecord::Observe { .. } => "observe",
        _ => "late",
    }
}

/// A community's violations, in calls and in the chat, newest first.
#[utoipa::path(get, path = "/communities/{community}/violations",
    params(("community" = String, Path, description = "The community's id"), PageQuery),
    responses((status = 200, body = proto::Violations), (status = 403, body = ApiError), (status = 404, body = ApiError)))]
async fn violations(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Path(g): Path<String>,
    Query(q): Query<PageQuery>,
) -> Result<Json<proto::Violations>, Answer> {
    let caller = api.caller(&headers)?;
    caller.may(Scope::Violations)?;
    let g = api.visible(&caller, &g)?;
    let details = caller.scopes.contains(&Scope::Details);
    let limit = q.limit.unwrap_or(50).max(1);
    let cursor = match q.cursor.as_deref() {
        None | Some("") => None,
        Some(c) => Some(Cursor(c.parse().map_err(|_| {
            Answer::error(StatusCode::BAD_REQUEST, "bad_request", "cursor is the next of an earlier page")
        })?)),
    };
    // Both lists are ordered by the event log's numbers: merged, newest first, a page at a time.
    let filter = SentenceFilter {
        guilds: Some(vec![g]),
        kind: SentenceKind::Violations,
        ..SentenceFilter::default()
    };
    let sentences = api
        .index
        .sentences(&filter, cursor, limit)
        .await
        .map_err(|e| Answer::store(&e))?
        .items;
    let mut chat_rows: Vec<ChatRow> = Vec::new();
    let mut chat_cursor = cursor;
    // Chat pages hold strikes too: read on until there are enough violations or none are left.
    loop {
        let page = api.index.chat(Some(g), chat_cursor, limit).await.map_err(|e| Answer::store(&e))?;
        chat_rows.extend(page.items.into_iter().filter(|c| c.record.decision.is_violation()));
        match page.next {
            Some(n) if chat_rows.len() < limit as usize => chat_cursor = Some(n),
            _ => break,
        }
    }
    let mut merged: Vec<(u64, proto::Violation)> = sentences
        .iter()
        .filter_map(|s| from_sentence(&api, s).map(|v| (s.seq, v)))
        .chain(chat_rows.iter().filter_map(|c| from_chat(&api, c, details).map(|v| (c.seq, v))))
        .collect();
    merged.sort_by(|a, b| b.0.cmp(&a.0));
    merged.truncate(limit as usize);
    // A full page may have more after it (the next one is older than its last).
    let next = (merged.len() == limit as usize)
        .then(|| merged.last().map(|(seq, _)| seq.to_string()))
        .flatten();
    Ok(Json(proto::Violations {
        items: merged.into_iter().map(|(_, v)| v).collect(),
        next,
    }))
}

fn from_sentence(api: &Api, s: &SentenceRow) -> Option<proto::Violation> {
    let r = &s.record;
    let (label, _, step, count) = r.decision.violation()?;
    Some(proto::Violation {
        id: r.id.to_string(),
        at: r.started,
        community: r.guild.to_string(),
        channel: r.channel.to_string(),
        person: api.person(r.guild, r.user),
        source: proto::Source::Call,
        label: label.key().to_owned(),
        count,
        step,
        decision: decision(&r.decision).to_owned(),
        text: None,
    })
}

fn from_chat(api: &Api, c: &ChatRow, details: bool) -> Option<proto::Violation> {
    let r = &c.record;
    let (label, _, step, count) = r.decision.violation()?;
    Some(proto::Violation {
        id: r.id.to_string(),
        at: r.at,
        community: r.guild.to_string(),
        channel: r.channel.to_string(),
        person: api.person(r.guild, r.user),
        source: proto::Source::Chat,
        label: label.key().to_owned(),
        count,
        step,
        decision: decision(&r.decision).to_owned(),
        text: details.then(|| r.text.clone()),
    })
}
