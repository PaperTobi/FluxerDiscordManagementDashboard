//! REST: `{api_public}/v1`, `Authorization: Bot <token>`, Fluxer's rate-limit headers and 429 answers honoured.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use pb_domain::{ChannelId, GuildId, MessageId, UserId};
use pb_fluxer_api::{
    Application, Attachment, Endpoints, ErrorKind, FluxerError, Member, MemberPatch, OutgoingMessage, User,
};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::wire;

/// The one HTTP client of every Fluxer call (connections and TLS sessions are shared).
pub(crate) fn http() -> Result<reqwest::Client, FluxerError> {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = HTTP.get() {
        return Ok(c.clone());
    }
    let tls = pb_tls::client_config().map_err(|e| FluxerError::new(ErrorKind::Network, e.to_string()))?;
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured((*tls).clone())
        .user_agent(concat!(
            "ProfanityWatchBot (https://github.com/PaperTobi/FluxerDiscordManagementDashboard, ",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| FluxerError::new(ErrorKind::Network, e.to_string()))?;
    Ok(HTTP.get_or_init(|| client).clone())
}

/// How hard a call tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Persistence {
    /// Wait out rate limits, outages and server errors, however long (messages are never dropped).
    UntilDone,
    /// Wait out rate limits; give up on network or server trouble after this many attempts.
    Attempts(u32),
}

/// Global requests per second (Fluxer: 50).
const GLOBAL_PER_SECOND: usize = 50;

#[derive(Debug, Default)]
struct Limits {
    /// Per route: requests left and when the bucket refills.
    buckets: HashMap<String, (u32, Instant)>,
    global_until: Option<Instant>,
    recent: VecDeque<Instant>,
}

/// The REST client.
pub(crate) struct Rest {
    http: reqwest::Client,
    ep: Endpoints,
    /// The bot's token (`None`: a client for the web login's OAuth2 calls only).
    token: Option<SecretString>,
    limits: Mutex<Limits>,
}

impl std::fmt::Debug for Rest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rest")
            .field("api", &self.ep.api.as_str())
            .finish_non_exhaustive()
    }
}

/// Who a call is made as.
#[derive(Clone, Copy)]
enum Auth<'a> {
    /// The bot (its token).
    Bot,
    /// Nobody (the OAuth2 token exchange authenticates with the client secret in the form).
    Anonymous,
    /// A logged-in person's OAuth2 access token.
    Bearer(&'a str),
}

/// The rate-limit route: the path with ids replaced, except the first (major) one after `channels/` or `guilds/`.
fn route(method: &reqwest::Method, path: &str) -> String {
    let mut prev = "";
    let mut major_done = false;
    let segs: Vec<String> = path
        .split('/')
        .map(|seg| {
            let is_id = !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_digit());
            let out = if is_id && !major_done && matches!(prev, "channels" | "guilds") {
                major_done = true;
                seg.to_owned()
            } else if is_id {
                ":id".to_owned()
            } else {
                seg.to_owned()
            };
            prev = seg;
            out
        })
        .collect();
    format!("{} {}", method.as_str(), segs.join("/"))
}

/// Seconds Fluxer asked to wait, as a duration: none when negative, a second when not a number or endless.
fn wait_secs(secs: f64) -> Duration {
    if secs <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f64(secs).unwrap_or(Duration::from_secs(1))
}

/// A `Retry-After` header (whole seconds).
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_millis(500)
        .saturating_mul(1 << attempt.min(7))
        .min(Duration::from_secs(60));
    base.mul_f64(0.75 + fastrand::f64() * 0.5)
}

pub(crate) fn error(status: u16, body: &Value) -> FluxerError {
    // API errors are `{code, message}`; OAuth2 errors are `{error, error_description}`.
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_owned);
    let code = text("code").or_else(|| text("error"));
    let message = text("message")
        .or_else(|| text("error_description"))
        .unwrap_or_else(|| body.to_string());
    let kind = match status {
        401 => ErrorKind::Unauthorized,
        403 => ErrorKind::Forbidden,
        404 => ErrorKind::NotFound,
        400 if matches!(
            code.as_deref(),
            Some("CANNOT_SEND_MESSAGES_TO_USER" | "MISSING_PERMISSIONS" | "MISSING_ACCESS")
        ) =>
        {
            ErrorKind::Forbidden
        }
        400..=499 => ErrorKind::BadRequest,
        _ => ErrorKind::Server,
    };
    FluxerError {
        kind,
        status: Some(status),
        code,
        message,
    }
}

enum Body<'a> {
    None,
    Json(&'a Value),
    /// JSON with the reason Fluxer's audit log shows for the change (ASCII; Fluxer stores it verbatim).
    Audited(&'a Value, &'a str),
    Form(&'a [(&'a str, &'a str)]),
    Multipart(&'a Value, &'a [Attachment]),
}

impl Rest {
    /// A client calling as the bot (`token`), or for OAuth2 calls only (`None`).
    pub(crate) fn new(ep: Endpoints, token: Option<SecretString>) -> Result<Rest, FluxerError> {
        Ok(Rest {
            http: http()?,
            ep,
            token,
            limits: Mutex::new(Limits::default()),
        })
    }

    pub(crate) fn endpoints(&self) -> &Endpoints {
        &self.ep
    }

    /// How long to wait before the next request on `route` (global pacing, global and bucket limits).
    fn wait_for(&self, route: &str) -> Duration {
        let now = Instant::now();
        let mut l = self.limits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut until = l.global_until.filter(|t| *t > now);
        if let Some(&(remaining, reset)) = l.buckets.get(route)
            && remaining == 0
            && reset > now
        {
            until = until.max(Some(reset));
        }
        while l
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1))
        {
            l.recent.pop_front();
        }
        if l.recent.len() >= GLOBAL_PER_SECOND
            && let Some(first) = l.recent.front()
        {
            until = until.max(Some(*first + Duration::from_secs(1)));
        }
        match until {
            Some(t) => t.duration_since(now),
            None => {
                l.recent.push_back(now);
                Duration::ZERO
            }
        }
    }

    fn learn(&self, route: &str, headers: &reqwest::header::HeaderMap) {
        let num = |k: &str| {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok())
        };
        if let (Some(remaining), Some(after)) = (num("x-ratelimit-remaining"), num("x-ratelimit-reset-after")) {
            let reset = Instant::now() + wait_secs(after);
            let left = remaining.max(0.0) as u32;
            self.limits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .buckets
                .insert(route.to_owned(), (left, reset));
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        url: String,
        path_for_route: &str,
        body: Body<'_>,
        auth: Auth<'_>,
        p: Persistence,
    ) -> Result<Value, FluxerError> {
        let route = route(&method, path_for_route);
        let mut attempt = 0u32;
        loop {
            loop {
                let wait = self.wait_for(&route);
                if wait.is_zero() {
                    break;
                }
                tokio::time::sleep(wait).await;
            }
            let mut req = self.http.request(method.clone(), &url);
            let credential = match auth {
                Auth::Bot => self.token.as_ref().map(|t| format!("Bot {}", t.expose_secret())),
                Auth::Bearer(t) => Some(format!("Bearer {t}")),
                Auth::Anonymous => None,
            };
            if let Some(c) = credential {
                let mut value = HeaderValue::from_str(&c).map_err(|_| {
                    FluxerError::new(
                        ErrorKind::Unauthorized,
                        "the token has characters a header cannot carry",
                    )
                })?;
                value.set_sensitive(true);
                req = req.header(AUTHORIZATION, value);
            }
            req = match &body {
                Body::None => req,
                Body::Json(v) => req.json(v),
                Body::Audited(v, reason) => req.json(v).header("X-Audit-Log-Reason", *reason),
                Body::Form(f) => req.form(f),
                Body::Multipart(payload, files) => {
                    let mut form = reqwest::multipart::Form::new().text("payload_json", payload.to_string());
                    for (i, f) in files.iter().enumerate() {
                        let part = reqwest::multipart::Part::bytes(f.bytes.to_vec())
                            .file_name(f.filename.clone())
                            .mime_str(&f.media_type)
                            .map_err(|e| FluxerError::new(ErrorKind::BadRequest, e.to_string()))?;
                        form = form.part(format!("files[{i}]"), part);
                    }
                    req.multipart(form)
                }
            };
            let retry_network = |attempt: u32| match p {
                Persistence::UntilDone => true,
                Persistence::Attempts(n) => attempt + 1 < n,
            };
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if retry_network(attempt) {
                        tracing::warn!(error = %e, route, "Fluxer could not be reached; trying again");
                        tokio::time::sleep(backoff(attempt)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(FluxerError::new(ErrorKind::Network, e.to_string()));
                }
            };
            let status = resp.status().as_u16();
            let headers = resp.headers().clone();
            self.learn(&route, &headers);
            let text = resp.text().await.unwrap_or_default();
            let value: Value = if text.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap_or(Value::String(text))
            };
            if status == 429 {
                let after = value
                    .get("retry_after")
                    .and_then(Value::as_f64)
                    .map(wait_secs)
                    .or_else(|| retry_after(&headers))
                    .unwrap_or(Duration::from_secs(1))
                    .max(Duration::from_millis(50));
                let until = Instant::now() + after;
                let global = value.get("global").and_then(Value::as_bool).unwrap_or(false)
                    || headers
                        .get("x-ratelimit-global")
                        .is_some_and(|v| v.as_bytes() == b"true");
                {
                    let mut l = self.limits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if global {
                        l.global_until = Some(until);
                    } else {
                        l.buckets.insert(route.clone(), (0, until));
                    }
                }
                tracing::debug!(route, ?after, global, "rate limited");
                continue;
            }
            if status >= 500 {
                if retry_network(attempt) {
                    tokio::time::sleep(retry_after(&headers).unwrap_or_else(|| backoff(attempt))).await;
                    attempt += 1;
                    continue;
                }
                return Err(error(status, &value));
            }
            if (200..300).contains(&status) {
                return Ok(value);
            }
            return Err(error(status, &value));
        }
    }

    async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Body<'_>,
        p: Persistence,
    ) -> Result<Value, FluxerError> {
        self.call(method, self.ep.rest(path), path, body, Auth::Bot, p).await
    }

    pub(crate) async fn application(&self) -> Result<Application, FluxerError> {
        let v = self
            .api(
                reqwest::Method::GET,
                "/applications/@me",
                Body::None,
                Persistence::Attempts(3),
            )
            .await?;
        Ok(Application {
            id: wire::id(v.get("id"))
                .ok_or_else(|| FluxerError::new(ErrorKind::Server, "the application has no id"))?,
            name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            owner: v.get("owner").and_then(wire::user),
            redirect_uris: v
                .get("redirect_uris")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
        })
    }

    pub(crate) async fn me(&self) -> Result<User, FluxerError> {
        let v = self
            .api(reqwest::Method::GET, "/users/@me", Body::None, Persistence::Attempts(3))
            .await?;
        wire::user(&v).ok_or_else(|| FluxerError::new(ErrorKind::Server, "an unreadable /users/@me"))
    }

    /// One message (content within the limit; the caller splits longer text).
    async fn create_one(
        &self,
        channel: ChannelId,
        m: &OutgoingMessage,
        content: &str,
        files: &[Attachment],
        nonce: &str,
    ) -> Result<MessageId, FluxerError> {
        let mut payload = json!({
            "content": content,
            "nonce": nonce,
            "allowed_mentions": { "parse": [], "users": m.ping.iter().map(ToString::to_string).collect::<Vec<_>>(), "replied_user": false },
        });
        if let Some(r) = m.reply_to {
            payload["message_reference"] = json!({ "message_id": r.to_string() });
        }
        if !files.is_empty() {
            payload["attachments"] = Value::Array(
                files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| json!({"id": i, "filename": f.filename}))
                    .collect(),
            );
        }
        let path = format!("/channels/{channel}/messages");
        let path = path.as_str();
        let send = |payload: Value| async move {
            let body = if files.is_empty() {
                Body::Json(&payload)
            } else {
                Body::Multipart(&payload, files)
            };
            self.api(reqwest::Method::POST, path, body, Persistence::UntilDone)
                .await
        };
        let result = match send(payload.clone()).await {
            // The message answered is gone (or too old to reference without Read Message History): send it plainly.
            Err(e) if m.reply_to.is_some() && (e.kind == ErrorKind::NotFound || e.is_code("UNKNOWN_MESSAGE")) => {
                let mut plain = payload;
                if let Some(o) = plain.as_object_mut() {
                    o.remove("message_reference");
                }
                send(plain).await
            }
            other => other,
        }?;
        wire::id(result.get("id"))
            .map(MessageId)
            .ok_or_else(|| FluxerError::new(ErrorKind::Server, "the created message has no id"))
    }

    /// Sends `m` to a channel; text longer than a message holds goes out as several messages (split at line
    /// breaks), files with the last. Each message carries its own nonce, kept when it is sent again after an error,
    /// so Fluxer creates it once. Returns the first message's id.
    pub(crate) async fn create_message(
        &self,
        channel: ChannelId,
        m: &OutgoingMessage,
    ) -> Result<MessageId, FluxerError> {
        let mut parts = split(&m.content, MESSAGE_UNITS);
        if parts.is_empty() {
            if m.files.is_empty() {
                return Err(FluxerError::new(ErrorKind::BadRequest, "an empty message"));
            }
            parts.push(String::new());
        }
        let mut first = None;
        let last = parts.len() - 1;
        for (i, part) in parts.iter().enumerate() {
            let files: &[Attachment] = if i == last { &m.files } else { &[] };
            let nonce: String = std::iter::repeat_with(fastrand::alphanumeric)
                .take(NONCE_CHARS)
                .collect();
            let id = self.create_one(channel, m, part, files, &nonce).await?;
            first.get_or_insert(id);
        }
        first.ok_or_else(|| FluxerError::new(ErrorKind::BadRequest, "an empty message"))
    }

    pub(crate) async fn open_dm(&self, user: UserId) -> Result<ChannelId, FluxerError> {
        let body = json!({ "recipient_id": user.to_string() });
        let v = self
            .api(
                reqwest::Method::POST,
                "/users/@me/channels",
                Body::Json(&body),
                Persistence::UntilDone,
            )
            .await?;
        wire::id(v.get("id"))
            .map(ChannelId)
            .ok_or_else(|| FluxerError::new(ErrorKind::Server, "the direct-message channel has no id"))
    }

    pub(crate) async fn react(&self, channel: ChannelId, message: MessageId, emoji: &str) -> Result<(), FluxerError> {
        let e: String = url::form_urlencoded::byte_serialize(emoji.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        let path = format!("/channels/{channel}/messages/{message}/reactions/{e}/@me");
        self.api(reqwest::Method::PUT, &path, Body::None, Persistence::Attempts(3))
            .await
            .map(|_| ())
    }

    pub(crate) async fn patch_member(&self, guild: GuildId, user: UserId, p: &MemberPatch) -> Result<(), FluxerError> {
        let mut body = serde_json::Map::new();
        if let Some(m) = p.mute {
            body.insert("mute".into(), json!(m));
        }
        if p.disconnect {
            body.insert("channel_id".into(), Value::Null);
        }
        if let Some(t) = &p.timeout_until {
            body.insert(
                "communication_disabled_until".into(),
                t.map_or(Value::Null, |t| json!(t.to_string())),
            );
        }
        let reason = p.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
        // A time-out takes its reason in the body, in full (as Fluxer's own app sends it); every other change takes it
        // in the X-Audit-Log-Reason header, which carries only visible ASCII.
        let header: Option<String> = match reason {
            Some(r) if matches!(p.timeout_until, Some(Some(_))) => {
                body.insert("timeout_reason".into(), json!(r));
                None
            }
            Some(r) => Some(r.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect()),
            None => None,
        };
        let body = Value::Object(body);
        let path = format!("/guilds/{guild}/members/{user}");
        self.api(
            reqwest::Method::PATCH,
            &path,
            match &header {
                Some(r) if !r.is_empty() => Body::Audited(&body, r),
                _ => Body::Json(&body),
            },
            Persistence::Attempts(3),
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn member(&self, guild: GuildId, user: UserId) -> Result<Option<Member>, FluxerError> {
        let path = format!("/guilds/{guild}/members/{user}");
        match self
            .api(reqwest::Method::GET, &path, Body::None, Persistence::Attempts(3))
            .await
        {
            Ok(v) => Ok(wire::member(&v)),
            Err(e) if e.kind == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// OAuth2: exchanges an authorization code (PKCE) for an access token.
    pub(crate) async fn oauth_token(
        &self,
        client_id: u64,
        secret: &str,
        code: &str,
        verifier: &str,
        redirect: &str,
    ) -> Result<String, FluxerError> {
        let id = client_id.to_string();
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("client_id", id.as_str()),
            ("client_secret", secret),
            ("code_verifier", verifier),
        ];
        let v = self
            .call(
                reqwest::Method::POST,
                self.ep.rest("/oauth2/token"),
                "/oauth2/token",
                Body::Form(&form),
                Auth::Anonymous,
                Persistence::Attempts(2),
            )
            .await
            .map_err(|mut e| {
                if e.status == Some(400) {
                    e.kind = ErrorKind::Unauthorized;
                }
                e
            })?;
        v.get("access_token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| FluxerError::new(ErrorKind::Server, "no access token"))
    }

    pub(crate) async fn oauth_userinfo(&self, access_token: &str) -> Result<Value, FluxerError> {
        self.call(
            reqwest::Method::GET,
            self.ep.rest("/oauth2/userinfo"),
            "/oauth2/userinfo",
            Body::None,
            Auth::Bearer(access_token),
            Persistence::Attempts(2),
        )
        .await
    }
}

/// What a bot's message holds, in UTF-16 code units (Fluxer measures content as JavaScript does).
const MESSAGE_UNITS: usize = 4000;

/// Fluxer takes nonces of up to 32 characters.
const NONCE_CHARS: usize = 25;

/// Splits text into pieces of at most `limit` UTF-16 code units, at line breaks where possible (a longer line at
/// character boundaries). Pieces of nothing but white space are left out.
fn split(text: &str, limit: usize) -> Vec<String> {
    // A character takes up to two units.
    let limit = limit.max(2);
    let mut out = Vec::new();
    let mut keep = |piece: String| {
        if !piece.trim().is_empty() {
            out.push(piece);
        }
    };
    let mut cur = String::new();
    let mut cur_len = 0;
    for line in text.split_inclusive('\n') {
        let n = line.encode_utf16().count();
        if cur_len + n > limit && !cur.is_empty() {
            keep(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if n <= limit {
            cur.push_str(line);
            cur_len += n;
            continue;
        }
        for c in line.chars() {
            if cur_len + c.len_utf16() > limit {
                keep(std::mem::take(&mut cur));
                cur_len = 0;
            }
            cur.push(c);
            cur_len += c.len_utf16();
        }
    }
    keep(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_keep_the_major_id() {
        assert_eq!(
            route(&reqwest::Method::POST, "/channels/123/messages"),
            "POST /channels/123/messages"
        );
        assert_eq!(
            route(&reqwest::Method::PUT, "/channels/1/messages/2/reactions/x/@me"),
            "PUT /channels/1/messages/:id/reactions/x/@me"
        );
        assert_eq!(
            route(&reqwest::Method::PATCH, "/guilds/5/members/6"),
            "PATCH /guilds/5/members/:id"
        );
    }

    #[test]
    fn long_text_is_split_at_lines() {
        let text = "a".repeat(10) + "\n" + &"b".repeat(10) + "\n" + &"c".repeat(25);
        let parts = split(&text, 12);
        assert_eq!(
            parts,
            vec!["aaaaaaaaaa\n", "bbbbbbbbbb\n", "cccccccccccc", "cccccccccccc", "c"]
        );
        assert_eq!(parts.concat(), text);
        assert_eq!(split("short", 4000), vec!["short"]);
    }

    #[test]
    fn odd_waits_do_not_panic() {
        assert_eq!(wait_secs(-3.0), Duration::ZERO);
        assert_eq!(wait_secs(f64::NAN), Duration::from_secs(1));
        assert_eq!(wait_secs(f64::INFINITY), Duration::from_secs(1));
        assert_eq!(wait_secs(0.25), Duration::from_millis(250));
    }

    #[test]
    fn pieces_are_measured_in_utf16_units_and_never_blank() {
        // An emoji takes two units: three fit in six, and none is cut in half.
        let parts = split("😀😀😀😀", 6);
        assert_eq!(parts, vec!["😀😀😀", "😀"]);
        // The line break after a full piece does not become a message of its own.
        assert_eq!(split("abcd\n", 4), vec!["abcd"]);
        assert_eq!(split("ab\n\n\ncd", 3), vec!["ab\n", "cd"]);
        assert!(split(" \n ", 4000).is_empty());
    }
}
