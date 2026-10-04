//! A fake Fluxer for tests: `/.well-known/fluxer`, the REST routes the bot uses, the gateway (Hello, Identify, READY and
//! the community burst, heartbeats, Resume with a replay buffer, op 7/9, close codes), voice joins with Fluxer's
//! pending-then-confirmed semantics, and the OAuth2 code exchange. It speaks the JSON of docs/fluxer-api.md, so the
//! client is tested against the documented wire format, and tests drive people, messages and outages through
//! [`FakeFluxer`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, patch, post, put};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::Digest as _;
use tokio::sync::mpsc;

/// Fluxer's heartbeat interval.
const HEARTBEAT_MS: u64 = 41_250;
/// How long Fluxer keeps a dropped session for a resume.
const RESUME_WINDOW: Duration = Duration::from_secs(60);

/// Makes a LiveKit grant for a bot connection: (endpoint, token).
pub type GrantFn = Arc<dyn Fn(u64, u64, u64, &str) -> (String, String) + Send + Sync>;

/// How the fake behaves.
#[derive(Clone)]
pub struct FakeConfig {
    /// The accepted bot token (`<application id>.<secret>`).
    pub token: String,
    pub app_id: u64,
    pub bot_id: u64,
    pub owner_id: u64,
    /// A join that is not confirmed within this time is dropped (Fluxer: 30 s).
    pub pending_timeout: Duration,
    pub grant: GrantFn,
    pub client_secret: String,
    /// Where to listen (default: a free local port). An unspecified address (0.0.0.0) is announced as `localhost`.
    pub bind: std::net::SocketAddr,
    /// How long a member search (op 8) takes to answer.
    pub search_delay: Duration,
}

impl std::fmt::Debug for FakeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeConfig")
            .field("app_id", &self.app_id)
            .field("bot_id", &self.bot_id)
            .finish_non_exhaustive()
    }
}

impl Default for FakeConfig {
    fn default() -> Self {
        FakeConfig {
            token: "1000.secret".into(),
            app_id: 1000,
            bot_id: 1001,
            owner_id: 1002,
            pending_timeout: Duration::from_secs(30),
            grant: Arc::new(|g, c, _bot, conn| ("wss://livekit.invalid".into(), format!("grant-{g}-{c}-{conn}"))),
            client_secret: "client-secret".into(),
            bind: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            search_delay: Duration::ZERO,
        }
    }
}

/// A message the bot sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub id: u64,
    pub channel: u64,
    pub payload: Value,
    /// (file name, bytes)
    pub files: Vec<(String, Vec<u8>)>,
}

impl Sent {
    pub fn content(&self) -> &str {
        self.payload.get("content").and_then(Value::as_str).unwrap_or_default()
    }
}

struct Session {
    tx: Option<mpsc::UnboundedSender<Out>>,
    seq: u64,
    buffer: Vec<(u64, Value)>,
    detached: Option<Instant>,
}

enum Out {
    Frame(Value),
    Close(u16),
}

struct Conn {
    guild: u64,
    channel: Option<u64>,
    user: u64,
    pending: Option<Instant>,
    session: Option<String>,
    self_mute: bool,
    self_deaf: bool,
}

#[derive(Default)]
struct Guild {
    name: String,
    owner: u64,
    roles: Vec<Value>,
    channels: Vec<Value>,
    members: BTreeMap<u64, Vec<u64>>,
}

/// An OAuth2 authorization code, as issued.
struct OAuthCode {
    user: u64,
    redirect_uri: String,
    /// The PKCE challenge and its method (`S256` or `plain`).
    challenge: Option<(String, String)>,
}

/// A member change the bot asked for.
#[derive(Debug, Clone)]
pub struct Patch {
    pub guild: u64,
    pub user: u64,
    pub body: Value,
    /// The reason the audit log shows.
    pub reason: Option<String>,
}

#[derive(Default)]
struct World {
    guilds: BTreeMap<u64, Guild>,
    users: BTreeMap<u64, Value>,
    voice: BTreeMap<String, Conn>,
    sessions: HashMap<String, Session>,
    sent: Vec<Sent>,
    reactions: Vec<(u64, u64, String)>,
    patches: Vec<Patch>,
    presences: Vec<Value>,
    voice_updates: Vec<Value>,
    dms: BTreeMap<u64, u64>,
    refuse_dms: Vec<u64>,
    /// When the latest member searches came (Fluxer's rate limit).
    searches: VecDeque<Instant>,
    /// New messages whose answer is lost on the way back (the message is created all the same).
    lose_answers: u32,
    next_id: u64,
    rate_limits: VecDeque<(String, f64, bool)>,
    /// Codes not exchanged yet (each works once).
    oauth_codes: HashMap<String, OAuthCode>,
    /// Who is logged in to Fluxer in "the browser" (approves logins at once); `None` = nobody.
    browser_user: Option<u64>,
    /// Messages people wrote (they can be replied to).
    said: Vec<u64>,
    /// The application's registered OAuth2 redirect addresses.
    redirect_uris: Vec<String>,
}

struct Shared {
    cfg: FakeConfig,
    world: Mutex<World>,
    base: String,
}

/// A running fake Fluxer.
#[derive(Clone)]
pub struct FakeFluxer {
    shared: Arc<Shared>,
    addr: SocketAddr,
}

impl std::fmt::Debug for FakeFluxer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeFluxer").field("addr", &self.addr).finish()
    }
}

fn s(id: u64) -> String {
    id.to_string()
}

impl Shared {
    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl World {
    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn user_json(&self, id: u64) -> Value {
        self.users
            .get(&id)
            .cloned()
            .unwrap_or_else(|| json!({"id": s(id), "username": format!("user{id}")}))
    }

    fn voice_json(&self, conn: &str, c: &Conn) -> Value {
        // Fluxer puts the member (with the user) into voice states (`guild_voice_state:refresh_member`).
        let roles: Vec<String> = self
            .guilds
            .get(&c.guild)
            .and_then(|g| g.members.get(&c.user))
            .map(|r| r.iter().map(|x| s(*x)).collect())
            .unwrap_or_default();
        json!({
            "guild_id": s(c.guild), "channel_id": c.channel.map(s), "user_id": s(c.user), "connection_id": conn,
            "session_id": c.session, "self_mute": c.self_mute, "self_deaf": c.self_deaf, "mute": false, "deaf": false,
            "self_video": false, "self_stream": false, "suppress": false, "e2ee_capable": false, "version": 1,
            "member": {"user": self.user_json(c.user), "roles": roles},
        })
    }

    /// A community as GUILD_CREATE carries it. At READY (`at_ready`) a member's user is only `{id}` (the users come
    /// in READY's `users`).
    fn guild_json(&self, gid: u64, bot: u64, at_ready: bool) -> Option<Value> {
        let g = self.guilds.get(&gid)?;
        let members: Vec<Value> = g
            .members
            .iter()
            .filter(|(u, _)| {
                **u == bot
                    || self
                        .voice
                        .values()
                        .any(|c| c.guild == gid && c.user == **u && c.channel.is_some())
            })
            .map(|(u, roles)| {
                let user = if at_ready {
                    json!({"id": s(*u)})
                } else {
                    self.user_json(*u)
                };
                json!({"user": user, "roles": roles.iter().map(|r| s(*r)).collect::<Vec<_>>()})
            })
            .collect();
        let voice: Vec<Value> = self
            .voice
            .iter()
            .filter(|(_, c)| c.guild == gid && c.channel.is_some())
            .map(|(k, c)| self.voice_json(k, c))
            .collect();
        Some(json!({
            "id": s(gid), "unavailable": false,
            "properties": {"name": g.name, "owner_id": s(g.owner), "icon": null},
            "roles": g.roles, "channels": g.channels, "members": members, "voice_states": voice,
        }))
    }

    /// Sends a dispatch to every session (buffered for resume while a session is detached).
    fn dispatch(&mut self, t: &str, d: &Value) {
        for sess in self.sessions.values_mut() {
            sess.seq += 1;
            let frame = json!({"op": 0, "t": t, "s": sess.seq, "d": d});
            sess.buffer.push((sess.seq, frame.clone()));
            if let Some(tx) = &sess.tx {
                let _ = tx.send(Out::Frame(frame));
            }
        }
    }

    fn dispatch_to(&mut self, session: &str, t: &str, d: &Value) {
        if let Some(sess) = self.sessions.get_mut(session) {
            sess.seq += 1;
            let frame = json!({"op": 0, "t": t, "s": sess.seq, "d": d});
            sess.buffer.push((sess.seq, frame.clone()));
            if let Some(tx) = &sess.tx {
                let _ = tx.send(Out::Frame(frame));
            }
        }
    }

    fn remove_conn(&mut self, conn: &str) {
        // A pending join was never announced, so its end is not either.
        if let Some(mut c) = self.voice.remove(conn).filter(|c| c.pending.is_none()) {
            c.channel = None;
            let v = self.voice_json(conn, &c);
            self.dispatch("VOICE_STATE_UPDATE", &v);
        }
    }

    /// Drops expired pending joins and the connections of sessions past their resume window.
    fn expire(&mut self, cfg: &FakeConfig) {
        let now = Instant::now();
        let dead_sessions: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.detached.is_some_and(|t| now.duration_since(t) > RESUME_WINDOW))
            .map(|(k, _)| k.clone())
            .collect();
        for k in &dead_sessions {
            self.sessions.remove(k);
        }
        let gone: Vec<String> = self
            .voice
            .iter()
            .filter(|(_, c)| {
                c.pending.is_some_and(|t| now.duration_since(t) > cfg.pending_timeout)
                    || c.session.as_ref().is_some_and(|s| dead_sessions.contains(s))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in gone {
            self.remove_conn(&k);
        }
    }
}

fn err(status: StatusCode, code: &str, message: &str) -> Response {
    (status, axum::Json(json!({"code": code, "message": message}))).into_response()
}

fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bot {}", shared.cfg.token))
}

/// Answers 429 when a rate limit was injected for this route.
fn limited(shared: &Shared, route: &str) -> Option<Response> {
    let mut w = shared.world();
    let pos = w
        .rate_limits
        .iter()
        .position(|(prefix, _, _)| route.starts_with(prefix.as_str()))?;
    let (_, after, global) = w.rate_limits.remove(pos)?;
    let mut r = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(json!({"code": "RATE_LIMITED", "message": "slow down", "global": global, "retry_after": after})),
    )
        .into_response();
    // The header in whole seconds (rounded up), as HTTP has it; the body keeps the fraction.
    r.headers_mut().insert(
        "retry-after",
        format!("{}", after.ceil().max(0.0) as u64)
            .parse()
            .unwrap_or_else(|_| unreachable!()),
    );
    Some(r)
}

macro_rules! guard {
    ($shared:expr, $headers:expr, $route:expr) => {
        if !authorized(&$shared, &$headers) {
            return err(StatusCode::UNAUTHORIZED, "INVALID_TOKEN", "invalid token");
        }
        if let Some(r) = limited(&$shared, &$route) {
            return r;
        }
    };
}

async fn well_known(State(sh): State<Arc<Shared>>) -> Response {
    axum::Json(json!({
        "api_code_version": 1,
        "endpoints": {"api": sh.base, "api_public": sh.base, "gateway": sh.base.replace("http://", "ws://") + "/gateway",
                      "media": format!("{}/media", sh.base), "webapp": format!("{}/app", sh.base)},
        "features": {"voice_enabled": true},
    }))
    .into_response()
}

async fn app_me(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    guard!(sh, headers, "GET /applications/@me");
    let w = sh.world();
    let owner = w.user_json(sh.cfg.owner_id);
    axum::Json(
        json!({"id": s(sh.cfg.app_id), "name": "Profanity Watch", "bot_public": false, "owner": owner,
                      "redirect_uris": w.redirect_uris}),
    )
    .into_response()
}

async fn users_me(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    guard!(sh, headers, "GET /users/@me");
    let me = sh.world().user_json(sh.cfg.bot_id);
    axum::Json(me).into_response()
}

async fn create_message(
    State(sh): State<Arc<Shared>>,
    Path(channel): Path<u64>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    guard!(sh, headers, format!("POST /channels/{channel}/messages"));
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let (payload, files) = if ct.starts_with("multipart/form-data") {
        let req = axum::http::Request::builder()
            .header("content-type", ct)
            .body(axum::body::Body::from(body))
            .unwrap_or_default();
        let Ok(mut mp) = <Multipart as axum::extract::FromRequest<()>>::from_request(req, &()).await else {
            return err(StatusCode::BAD_REQUEST, "INVALID_FORM_BODY", "bad multipart");
        };
        let (mut payload, mut files) = (Value::Null, Vec::new());
        while let Ok(Some(field)) = mp.next_field().await {
            let name = field.name().unwrap_or_default().to_owned();
            let filename = field.file_name().unwrap_or_default().to_owned();
            let bytes = field.bytes().await.unwrap_or_default().to_vec();
            if name == "payload_json" {
                payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            } else if name.starts_with("files[") {
                files.push((filename, bytes));
            }
        }
        (payload, files)
    } else {
        (serde_json::from_slice(&body).unwrap_or(Value::Null), Vec::new())
    };
    let mut w = sh.world();
    if w.dms.iter().any(|(u, c)| *c == channel && w.refuse_dms.contains(u)) {
        return err(
            StatusCode::BAD_REQUEST,
            "CANNOT_SEND_MESSAGES_TO_USER",
            "the user does not take direct messages",
        );
    }
    if let Some(r) = payload.pointer("/message_reference/message_id").and_then(Value::as_str) {
        let known = w.sent.iter().any(|m| s(m.id) == r) || w.said.iter().any(|m| s(*m) == r);
        if !known {
            return err(StatusCode::NOT_FOUND, "UNKNOWN_MESSAGE", "unknown message");
        }
    }
    let content = payload.get("content").and_then(Value::as_str).unwrap_or_default();
    // As Fluxer: white space alone is no content; length is counted in UTF-16 units (a bot's limit is 4000).
    if content.trim().is_empty() && files.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "CANNOT_SEND_EMPTY_MESSAGE",
            "cannot send an empty message",
        );
    }
    if content.encode_utf16().count() > 4000 {
        return err(StatusCode::BAD_REQUEST, "INVALID_FORM_BODY", "content too long");
    }
    // A nonce seen before from this bot answers with the message it created.
    let nonce = payload.get("nonce").and_then(Value::as_str);
    if let Some(m) = nonce.and_then(|n| {
        w.sent
            .iter()
            .find(|m| m.payload.get("nonce").and_then(Value::as_str) == Some(n))
    }) {
        if m.channel != channel {
            return err(StatusCode::NOT_FOUND, "UNKNOWN_MESSAGE", "unknown message");
        }
        return axum::Json(json!({"id": s(m.id), "channel_id": s(channel), "content": m.content()})).into_response();
    }
    let id = w.id();
    w.sent.push(Sent {
        id,
        channel,
        payload: payload.clone(),
        files,
    });
    if w.lose_answers > 0 {
        w.lose_answers -= 1;
        return err(StatusCode::BAD_GATEWAY, "BAD_GATEWAY", "the answer was lost");
    }
    axum::Json(json!({"id": s(id), "channel_id": s(channel), "content": content})).into_response()
}

async fn react(
    State(sh): State<Arc<Shared>>,
    Path((c, m, emoji)): Path<(u64, u64, String)>,
    headers: HeaderMap,
) -> Response {
    guard!(sh, headers, format!("PUT /channels/{c}/messages/{m}/reactions"));
    sh.world().reactions.push((c, m, emoji));
    StatusCode::NO_CONTENT.into_response()
}

async fn open_dm(State(sh): State<Arc<Shared>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    guard!(sh, headers, "POST /users/@me/channels");
    let Some(user) = body
        .get("recipient_id")
        .and_then(Value::as_str)
        .and_then(|u| u.parse::<u64>().ok())
    else {
        return err(StatusCode::BAD_REQUEST, "INVALID_FORM_BODY", "recipient_id");
    };
    let mut w = sh.world();
    let channel = match w.dms.get(&user) {
        Some(c) => *c,
        None => {
            let c = w.id();
            w.dms.insert(user, c);
            c
        }
    };
    axum::Json(json!({"id": s(channel), "type": 1, "recipients": [w.user_json(user)]})).into_response()
}

async fn member_get(State(sh): State<Arc<Shared>>, Path((g, u)): Path<(u64, u64)>, headers: HeaderMap) -> Response {
    guard!(sh, headers, format!("GET /guilds/{g}/members"));
    let w = sh.world();
    match w.guilds.get(&g).and_then(|gg| gg.members.get(&u)) {
        Some(roles) => axum::Json(json!({"user": w.user_json(u), "roles": roles.iter().map(|r| s(*r)).collect::<Vec<_>>(), "mute": false, "deaf": false})).into_response(),
        None => err(StatusCode::NOT_FOUND, "UNKNOWN_MEMBER", "unknown member"),
    }
}

async fn member_patch(
    State(sh): State<Arc<Shared>>,
    Path((g, u)): Path<(u64, u64)>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    guard!(sh, headers, format!("PATCH /guilds/{g}/members"));
    let mut w = sh.world();
    if body.get("channel_id").is_some_and(Value::is_null) {
        let conns: Vec<String> = w
            .voice
            .iter()
            .filter(|(_, c)| c.guild == g && c.user == u)
            .map(|(k, _)| k.clone())
            .collect();
        if conns.is_empty() {
            return err(StatusCode::BAD_REQUEST, "USER_NOT_IN_VOICE", "the user is not in voice");
        }
        for k in conns {
            w.remove_conn(&k);
        }
    }
    // The reason Fluxer's audit log keeps: the header, else a time-out's own reason.
    let header = headers
        .get("x-audit-log-reason")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let timeout_reason = body
        .get("communication_disabled_until")
        .and_then(|_| body.get("timeout_reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let reason = header.or(timeout_reason).map(str::to_owned);
    w.patches.push(Patch {
        guild: g,
        user: u,
        body,
        reason,
    });
    axum::Json(json!({"user": w.user_json(u), "roles": []})).into_response()
}

/// Answers member searches one after another: `d` first, then whatever is waiting in `slot` when it is done.
async fn run_searches(sh: Arc<Shared>, session: String, mut d: Value, slot: Arc<Mutex<(bool, Option<Value>)>>) {
    loop {
        tokio::time::sleep(sh.cfg.search_delay).await;
        answer_search(&sh, &session, &d);
        let mut s = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match s.1.take() {
            Some(next) => d = next,
            None => {
                s.0 = false;
                return;
            }
        }
    }
}

/// A name prefix in one community, answered in one chunk with the nonce.
fn answer_search(sh: &Shared, session: &str, d: &Value) {
    let g = d
        .get("guild_id")
        .and_then(Value::as_str)
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(0);
    let query = d
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let limit = d.get("limit").and_then(Value::as_u64).unwrap_or(0).min(100) as usize;
    let mut w = sh.world();
    let members: Vec<Value> = w
        .guilds
        .get(&g)
        .map(|gg| {
            gg.members
                .iter()
                .map(|(u, roles)| (*u, roles.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(u, roles)| json!({"user": w.user_json(u), "roles": roles.iter().map(|r| s(*r)).collect::<Vec<_>>()}))
        .filter(|m| {
            let name = |k: &str| m.pointer(k).and_then(Value::as_str).unwrap_or_default().to_lowercase();
            query.is_empty()
                || name("/user/username").starts_with(&query)
                || name("/user/global_name").starts_with(&query)
        })
        .take(if limit == 0 { 100 } else { limit })
        .collect();
    let mut chunk = json!({"guild_id": s(g), "members": members, "chunk_index": 0, "chunk_count": 1});
    if let Some(n) = d.get("nonce") {
        chunk["nonce"] = n.clone();
    }
    w.dispatch_to(session, "GUILD_MEMBERS_CHUNK", &chunk);
}

/// The consent step, approved at once for the browser's user (Fluxer shows a consent page here).
async fn oauth_authorize(
    State(sh): State<Arc<Shared>>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let Some(redirect) = q.get("redirect_uri").and_then(|r| url::Url::parse(r).ok()) else {
        return err(StatusCode::BAD_REQUEST, "INVALID_FORM_BODY", "redirect_uri");
    };
    if q.get("client_id") != Some(&sh.cfg.app_id.to_string()) || q.get("response_type").is_some_and(|t| t != "code") {
        return err(StatusCode::BAD_REQUEST, "INVALID_FORM_BODY", "client_id");
    }
    if !sh.world().redirect_uris.iter().any(|r| r.as_str() == redirect.as_str()) {
        return err(
            StatusCode::BAD_REQUEST,
            "INVALID_FORM_BODY",
            "redirect_uri is not registered",
        );
    }
    let redirect_text = q.get("redirect_uri").cloned().unwrap_or_default();
    let mut back = redirect;
    {
        let mut w = sh.world();
        let mut pairs = back.query_pairs_mut();
        match w.browser_user {
            Some(user) => {
                let code = format!("code{}", fastrand::u64(..));
                let challenge = q.get("code_challenge").map(|c| {
                    let method = q
                        .get("code_challenge_method")
                        .cloned()
                        .unwrap_or_else(|| "plain".into());
                    (c.clone(), method)
                });
                w.oauth_codes.insert(
                    code.clone(),
                    OAuthCode {
                        user,
                        redirect_uri: redirect_text.clone(),
                        challenge,
                    },
                );
                pairs.append_pair("code", &code);
            }
            None => {
                pairs
                    .append_pair("error", "access_denied")
                    .append_pair("error_description", "nobody is logged in");
            }
        }
        if let Some(st) = q.get("state") {
            pairs.append_pair("state", st);
        }
    }
    axum::response::Redirect::to(back.as_str()).into_response()
}

/// The code exchange, as Fluxer checks it: the client's secret, then the code (it works once), the redirect address
/// it was issued for, and the PKCE verifier when the code has a challenge.
async fn oauth_token(State(sh): State<Arc<Shared>>, axum::Form(form): axum::Form<HashMap<String, String>>) -> Response {
    let invalid = |error: &str| (StatusCode::BAD_REQUEST, axum::Json(json!({"error": error}))).into_response();
    if form.get("client_id") != Some(&sh.cfg.app_id.to_string())
        || form.get("client_secret") != Some(&sh.cfg.client_secret)
    {
        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"error": "invalid_client"}))).into_response();
    }
    let Some(code) = form.get("code").and_then(|c| sh.world().oauth_codes.remove(c)) else {
        return invalid("invalid_grant");
    };
    if form.get("redirect_uri").map_or("", String::as_str) != code.redirect_uri {
        return invalid("invalid_grant");
    }
    if let Some((challenge, method)) = &code.challenge {
        let Some(verifier) = form.get("code_verifier") else {
            return invalid("invalid_grant");
        };
        let expected = if method == "S256" {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
        } else {
            verifier.clone()
        };
        if &expected != challenge {
            return invalid("invalid_grant");
        }
    }
    let user = code.user;
    axum::Json(json!({"access_token": format!("at-{user}"), "token_type": "Bearer", "expires_in": 604800, "scope": "identify"})).into_response()
}

async fn oauth_userinfo(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer at-"))
        .and_then(|u| u.parse::<u64>().ok());
    match token {
        Some(u) => {
            let mut user = sh.world().user_json(u);
            user["sub"] = json!(s(u));
            axum::Json(user).into_response()
        }
        None => err(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "bad token"),
    }
}

async fn gateway(
    State(sh): State<Arc<Shared>>,
    ws: WebSocketUpgrade,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let version_ok = q.get("v").map(String::as_str) == Some("1");
    ws.on_upgrade(move |socket| socket_task(sh, socket, version_ok))
}

async fn socket_task(sh: Arc<Shared>, socket: WebSocket, version_ok: bool) {
    let (mut sink, mut stream) = socket.split();
    if !version_ok {
        let _ = sink
            .send(Message::Close(Some(CloseFrame {
                code: 4012,
                reason: "invalid API version".into(),
            })))
            .await;
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let _ = tx.send(Out::Frame(json!({"op": 10, "d": {"heartbeat_interval": HEARTBEAT_MS}})));
    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let msg = match out {
                Out::Frame(v) => Message::Text(v.to_string().into()),
                Out::Close(code) => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code,
                            reason: "".into(),
                        })))
                        .await;
                    return;
                }
            };
            if sink.send(msg).await.is_err() {
                return;
            }
        }
    });
    let mut my_session: Option<String> = None;
    // (a search is running, the one waiting)
    let search_slot: Arc<Mutex<(bool, Option<Value>)>> = Arc::default();
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            if matches!(msg, Message::Close(_)) {
                break;
            }
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
            let _ = tx.send(Out::Close(4002));
            break;
        };
        let op = v.get("op").and_then(Value::as_u64);
        let d = v.get("d").cloned().unwrap_or(Value::Null);
        match op {
            Some(1) => {
                let _ = tx.send(Out::Frame(json!({"op": 11})));
            }
            Some(2) => {
                if d.get("token").and_then(Value::as_str) != Some(sh.cfg.token.as_str()) {
                    let _ = tx.send(Out::Close(4004));
                    break;
                }
                let id = format!("sess{}", fastrand::u64(..));
                let mut w = sh.world();
                w.sessions.insert(
                    id.clone(),
                    Session {
                        tx: Some(tx.clone()),
                        seq: 0,
                        buffer: Vec::new(),
                        detached: None,
                    },
                );
                let guild_ids: Vec<u64> = w.guilds.keys().copied().collect();
                let creates: Vec<Value> = guild_ids
                    .iter()
                    .filter_map(|g| w.guild_json(*g, sh.cfg.bot_id, true))
                    .collect();
                // The users of the members GUILD_CREATE lists, once each.
                let users: Vec<Value> = creates
                    .iter()
                    .flat_map(|g| g["members"].as_array().cloned().unwrap_or_default())
                    .filter_map(|m| {
                        m.pointer("/user/id")
                            .and_then(Value::as_str)
                            .and_then(|u| u.parse::<u64>().ok())
                    })
                    .collect::<std::collections::BTreeSet<u64>>()
                    .into_iter()
                    .map(|u| w.user_json(u))
                    .collect();
                let ready = json!({"session_id": id, "version": 1, "user": w.user_json(sh.cfg.bot_id), "users": users,
                                   "guilds": guild_ids.iter().map(|g| json!({"id": s(*g), "unavailable": true})).collect::<Vec<_>>()});
                // READY and the burst are not replayable (outside the buffer).
                if let Some(sess) = w.sessions.get_mut(&id) {
                    sess.seq += 1;
                    let _ = tx.send(Out::Frame(json!({"op": 0, "t": "READY", "s": sess.seq, "d": ready})));
                }
                for gj in creates {
                    if let Some(sess) = w.sessions.get_mut(&id) {
                        sess.seq += 1;
                        let _ = tx.send(Out::Frame(
                            json!({"op": 0, "t": "GUILD_CREATE", "s": sess.seq, "d": gj}),
                        ));
                    }
                }
                my_session = Some(id);
            }
            Some(6) => {
                let id = d
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let seq = d.get("seq").and_then(Value::as_u64).unwrap_or(0);
                let mut w = sh.world();
                let token_ok = d.get("token").and_then(Value::as_str) == Some(sh.cfg.token.as_str());
                // As Fluxer: a wrong token closes 4004; an unknown session is op 9; RESUMED carries the current sequence.
                if !token_ok {
                    let _ = tx.send(Out::Close(4004));
                    break;
                }
                match w.sessions.get_mut(&id) {
                    Some(sess) => {
                        sess.tx = Some(tx.clone());
                        sess.detached = None;
                        for (s_, f) in &sess.buffer {
                            if *s_ > seq {
                                let _ = tx.send(Out::Frame(f.clone()));
                            }
                        }
                        let _ = tx.send(Out::Frame(json!({"op": 0, "t": "RESUMED", "s": sess.seq, "d": {}})));
                        my_session = Some(id);
                    }
                    None => {
                        let _ = tx.send(Out::Frame(json!({"op": 9, "d": false})));
                    }
                }
            }
            Some(3) => sh.world().presences.push(d),
            // Request Guild Members, as Fluxer: one search at a time per connection; one that arrives meanwhile waits,
            // and a newer one replaces it (silently). More than 12 in 10 s are dropped (silently).
            Some(8) => {
                let Some(session) = my_session.clone() else {
                    let _ = tx.send(Out::Close(4003));
                    break;
                };
                {
                    let mut w = sh.world();
                    let now = Instant::now();
                    w.searches.retain(|t| now.duration_since(*t) < Duration::from_secs(10));
                    if w.searches.len() >= 12 {
                        continue;
                    }
                    w.searches.push_back(now);
                }
                let start = {
                    let mut slot = search_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if slot.0 {
                        slot.1 = Some(d);
                        None
                    } else {
                        slot.0 = true;
                        Some(d)
                    }
                };
                if let Some(d) = start {
                    tokio::spawn(run_searches(sh.clone(), session, d, search_slot.clone()));
                }
            }
            Some(4) => {
                let Some(session) = my_session.clone() else {
                    let _ = tx.send(Out::Close(4003));
                    break;
                };
                voice_update(&sh, &session, &d);
            }
            _ => {
                let _ = tx.send(Out::Close(4001));
                break;
            }
        }
    }
    if let Some(id) = my_session {
        let mut w = sh.world();
        if let Some(sess) = w.sessions.get_mut(&id)
            && sess.tx.as_ref().is_some_and(|t| t.same_channel(&tx))
        {
            sess.tx = None;
            sess.detached = Some(Instant::now());
        }
    }
    drop(tx);
    let _ = writer.await;
}

fn voice_update(sh: &Shared, session: &str, d: &Value) {
    let mut w = sh.world();
    w.voice_updates.push(d.clone());
    let guild = d
        .get("guild_id")
        .and_then(Value::as_str)
        .and_then(|g| g.parse::<u64>().ok())
        .unwrap_or(0);
    let channel = d
        .get("channel_id")
        .and_then(Value::as_str)
        .and_then(|c| c.parse::<u64>().ok());
    let conn = d.get("connection_id").and_then(Value::as_str).map(str::to_owned);
    match (channel, conn) {
        (Some(channel), None) => {
            let conn = format!("conn{}", fastrand::u32(..));
            let c = Conn {
                guild,
                channel: Some(channel),
                user: sh.cfg.bot_id,
                pending: Some(Instant::now()),
                session: Some(session.to_owned()),
                self_mute: false,
                self_deaf: false,
            };
            // Pending, not announced, until confirmed (fluxer_gateway guild_voice_connection_join.erl).
            w.voice.insert(conn.clone(), c);
            let (endpoint, token) = (sh.cfg.grant)(guild, channel, sh.cfg.bot_id, &conn);
            let grant = json!({"token": token, "endpoint": endpoint, "connection_id": conn, "channel_id": s(channel), "guild_id": s(guild)});
            w.dispatch_to(session, "VOICE_SERVER_UPDATE", &grant);
        }
        (Some(channel), Some(conn)) => {
            let state = match w.voice.get_mut(&conn) {
                Some(c) if c.user == sh.cfg.bot_id => {
                    // The same channel confirms a pending join; another channel moves the connection.
                    c.pending = None;
                    c.channel = Some(channel);
                    Some(())
                }
                _ => None,
            };
            if state.is_some() {
                let v = w.voice.get(&conn).map(|c| w.voice_json(&conn, c));
                if let Some(v) = v {
                    w.dispatch("VOICE_STATE_UPDATE", &v);
                }
            }
        }
        (None, Some(conn)) => w.remove_conn(&conn),
        (None, None) => {}
    }
}

impl FakeFluxer {
    /// Starts on a free local port.
    pub async fn start(cfg: FakeConfig) -> FakeFluxer {
        let listener = tokio::net::TcpListener::bind(cfg.bind)
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener.local_addr().unwrap_or_else(|e| panic!("addr: {e}"));
        let shown = if addr.ip().is_unspecified() {
            format!("localhost:{}", addr.port())
        } else {
            addr.to_string()
        };
        let shared = Arc::new(Shared {
            cfg: cfg.clone(),
            world: Mutex::new(World {
                next_id: 9_000_000,
                ..World::default()
            }),
            base: format!("http://{shown}"),
        });
        {
            let mut w = shared.world();
            w.users.insert(
                cfg.bot_id,
                json!({"id": s(cfg.bot_id), "username": "watchbot", "bot": true}),
            );
            w.users.insert(
                cfg.owner_id,
                json!({"id": s(cfg.owner_id), "username": "owner", "global_name": "The Owner"}),
            );
        }
        let app = Router::new()
            .route("/.well-known/fluxer", get(well_known))
            .route("/v1/applications/@me", get(app_me))
            .route("/v1/users/@me", get(users_me))
            .route("/v1/channels/{c}/messages", post(create_message))
            .route("/v1/channels/{c}/messages/{m}/reactions/{e}/@me", put(react))
            .route("/v1/users/@me/channels", post(open_dm))
            .route("/v1/guilds/{g}/members/{u}", get(member_get).merge(patch(member_patch)))
            .route("/v1/oauth2/authorize", get(oauth_authorize))
            .route("/v1/oauth2/token", post(oauth_token))
            .route("/v1/oauth2/userinfo", get(oauth_userinfo))
            .route("/gateway", any(gateway))
            .with_state(shared.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let expirer = Arc::downgrade(&shared);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let Some(sh) = expirer.upgrade() else { return };
                let cfg = sh.cfg.clone();
                sh.world().expire(&cfg);
            }
        });
        FakeFluxer { shared, addr }
    }

    /// The instance address to give the client.
    pub fn url(&self) -> String {
        self.shared.base.clone()
    }

    pub fn config(&self) -> &FakeConfig {
        &self.shared.cfg
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.shared.world()
    }

    pub fn add_user(&self, id: u64, username: &str, global_name: Option<&str>) {
        self.world().users.insert(
            id,
            json!({"id": s(id), "username": username, "global_name": global_name}),
        );
    }

    /// A community with an @everyone role allowing `everyone` permissions.
    pub fn add_guild(&self, id: u64, name: &str, owner: u64, everyone: u64) {
        let mut w = self.world();
        let g = Guild {
            name: name.into(),
            owner,
            roles: vec![json!({"id": s(id), "name": "@everyone", "permissions": s(everyone), "position": 0})],
            ..Guild::default()
        };
        w.guilds.insert(id, g);
        let bot = self.shared.cfg.bot_id;
        if let Some(g) = w.guilds.get_mut(&id) {
            g.members.insert(bot, Vec::new());
        }
        if let Some(gj) = w.guild_json(id, bot, false) {
            w.dispatch("GUILD_CREATE", &gj);
        }
    }

    pub fn add_role(&self, guild: u64, id: u64, name: &str, permissions: u64) {
        let mut w = self.world();
        let role = json!({"id": s(id), "name": name, "permissions": s(permissions), "position": 1});
        if let Some(g) = w.guilds.get_mut(&guild) {
            g.roles.push(role.clone());
        }
        w.dispatch("GUILD_ROLE_CREATE", &json!({"guild_id": s(guild), "role": role}));
    }

    /// A channel of Fluxer's type `kind` (0 text, 2 voice, 4 category).
    pub fn add_channel(&self, guild: u64, id: u64, name: &str, kind: u16) {
        let mut w = self.world();
        let ch = json!({"id": s(id), "guild_id": s(guild), "name": name, "type": kind, "position": 0, "permission_overwrites": []});
        if let Some(g) = w.guilds.get_mut(&guild) {
            g.channels.push(ch.clone());
        }
        w.dispatch("CHANNEL_CREATE", &ch);
    }

    pub fn add_member(&self, guild: u64, user: u64, roles: &[u64]) {
        let mut w = self.world();
        if let Some(g) = w.guilds.get_mut(&guild) {
            g.members.insert(user, roles.to_vec());
        }
        let m = json!({"guild_id": s(guild), "user": w.user_json(user), "roles": roles.iter().map(|r| s(*r)).collect::<Vec<_>>()});
        w.dispatch("GUILD_MEMBER_ADD", &m);
    }

    /// A person joins a voice channel; returns their connection id.
    pub fn voice_join(&self, guild: u64, channel: u64, user: u64) -> String {
        let mut w = self.world();
        let conn = format!("human{}", fastrand::u32(..));
        let c = Conn {
            guild,
            channel: Some(channel),
            user,
            pending: None,
            session: None,
            self_mute: false,
            self_deaf: false,
        };
        let v = w.voice_json(&conn, &c);
        w.voice.insert(conn.clone(), c);
        w.dispatch("VOICE_STATE_UPDATE", &v);
        conn
    }

    /// A person mutes or deafens themselves.
    pub fn voice_self(&self, conn: &str, mute: bool, deaf: bool) {
        let mut w = self.world();
        let Some(c) = w.voice.get_mut(conn) else {
            return;
        };
        c.self_mute = mute;
        c.self_deaf = deaf;
        let v = w.voice.get(conn).map(|c| w.voice_json(conn, c));
        if let Some(v) = v {
            w.dispatch("VOICE_STATE_UPDATE", &v);
        }
    }

    pub fn voice_leave(&self, conn: &str) {
        self.world().remove_conn(conn);
    }

    /// The bot's connections: (connection id, guild, channel, still pending).
    pub fn bot_connections(&self) -> Vec<(String, u64, u64, bool)> {
        let bot = self.shared.cfg.bot_id;
        self.world()
            .voice
            .iter()
            .filter(|(_, c)| c.user == bot)
            .filter_map(|(k, c)| Some((k.clone(), c.guild, c.channel?, c.pending.is_some())))
            .collect()
    }

    /// A chat message from `author` (with `roles`).
    pub fn say(&self, guild: u64, channel: u64, author: u64, roles: &[u64], content: &str) -> u64 {
        let mut w = self.world();
        let id = w.id();
        w.said.push(id);
        let m = json!({"id": s(id), "channel_id": s(channel), "guild_id": s(guild), "content": content, "author": w.user_json(author),
                       "member": {"roles": roles.iter().map(|r| s(*r)).collect::<Vec<_>>()}, "mentions": []});
        w.dispatch("MESSAGE_CREATE", &m);
        id
    }

    /// Closes every gateway socket with `code` (sessions stay resumable for the resume window).
    pub fn drop_connections(&self, code: u16) {
        let mut w = self.world();
        for sess in w.sessions.values_mut() {
            if let Some(tx) = sess.tx.take() {
                let _ = tx.send(Out::Close(code));
                sess.detached = Some(Instant::now());
            }
        }
    }

    /// Sends op 7 (reconnect) and closes with 4000, like Fluxer.
    pub fn ask_reconnect(&self) {
        let mut w = self.world();
        for sess in w.sessions.values_mut() {
            if let Some(tx) = sess.tx.take() {
                let _ = tx.send(Out::Frame(json!({"op": 7})));
                let _ = tx.send(Out::Close(4000));
                sess.detached = Some(Instant::now());
            }
        }
    }

    /// Forgets every session and closes their sockets with 4000: the client's Resume gets op 9.
    pub fn forget_sessions(&self) {
        let mut w = self.world();
        for (_, sess) in w.sessions.drain() {
            if let Some(tx) = sess.tx {
                let _ = tx.send(Out::Close(4000));
            }
        }
    }

    /// The next request whose route starts with `prefix` (e.g. `POST /channels/5/messages`) gets a 429.
    pub fn rate_limit_next(&self, prefix: &str, retry_after: f64, global: bool) {
        self.world()
            .rate_limits
            .push_back((prefix.to_owned(), retry_after, global));
    }

    /// The next new message is created but its answer is lost (a proxy failing on the way back).
    pub fn lose_next_answer(&self) {
        self.world().lose_answers += 1;
    }

    pub fn refuse_dms(&self, user: u64) {
        self.world().refuse_dms.push(user);
    }

    /// Registers an OAuth2 redirect address for the application (logins may only come back to these).
    pub fn add_redirect(&self, uri: &str) {
        self.world().redirect_uris.push(uri.to_owned());
    }

    /// Who is logged in to Fluxer in the browser (logins through `/oauth2/authorize` are approved for them).
    pub fn log_in_browser(&self, user: Option<u64>) {
        self.world().browser_user = user;
    }

    /// An OAuth2 code that logs `user` in, issued for `redirect_uri` with PKCE (`S256` of `verifier`).
    pub fn oauth_code(&self, user: u64, redirect_uri: &str, verifier: &str) -> String {
        use base64::Engine as _;
        let code = format!("code{}", fastrand::u64(..));
        let challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
        self.world().oauth_codes.insert(
            code.clone(),
            OAuthCode {
                user,
                redirect_uri: redirect_uri.to_owned(),
                challenge: Some((challenge, "S256".into())),
            },
        );
        code
    }

    pub fn sent(&self) -> Vec<Sent> {
        self.world().sent.clone()
    }

    pub fn reactions(&self) -> Vec<(u64, u64, String)> {
        self.world().reactions.clone()
    }

    pub fn patches(&self) -> Vec<Patch> {
        self.world().patches.clone()
    }

    pub fn presences(&self) -> Vec<Value> {
        self.world().presences.clone()
    }

    pub fn voice_updates(&self) -> Vec<Value> {
        self.world().voice_updates.clone()
    }

    /// The DM channel opened for `user`.
    pub fn dm_channel(&self, user: u64) -> Option<u64> {
        self.world().dms.get(&user).copied()
    }

    /// Waits until `f` holds (polling), or panics after `timeout`.
    pub async fn wait_until(&self, timeout: Duration, what: &str, f: impl Fn(&FakeFluxer) -> bool) {
        let end = Instant::now() + timeout;
        while !f(self) {
            assert!(Instant::now() < end, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
