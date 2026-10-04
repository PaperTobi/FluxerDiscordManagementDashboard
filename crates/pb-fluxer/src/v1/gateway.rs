//! The gateway connection: Hello, Identify or Resume, heartbeats, dispatches, reconnects with resume, close codes, the
//! pacing of presence (op 3) and voice-state (op 4) updates, and member searches (op 8) one at a time
//! (docs/fluxer-api.md).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use pb_domain::{GuildId, RoleId, UserId};
use pb_fluxer_api::{BotIdentity, ErrorKind, Fatal, FluxerError, GatewayEvent, Member};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use url::Url;

use super::wire;

/// Dispatches the bot never reads (fewer frames for Fluxer and for us).
pub(crate) const IGNORED_EVENTS: &[&str] = &[
    "TYPING_START",
    "MESSAGE_UPDATE",
    "MESSAGE_DELETE",
    "MESSAGE_DELETE_BULK",
    "MESSAGE_REACTION_ADD",
    "MESSAGE_REACTION_REMOVE",
    "MESSAGE_REACTION_REMOVE_ALL",
    "MESSAGE_REACTION_REMOVE_EMOJI",
    "PRESENCE_UPDATE",
];

/// Timings (tests shorten them).
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub hello_timeout: Duration,
    /// How long an Identify may wait for READY before the connection is tried again (Fluxer may hold it silently).
    pub ready_timeout: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// After close code 4008 (rate limited).
    pub rate_limit_backoff: Duration,
    /// A connection that lasted this long resets the backoff.
    pub healthy_after: Duration,
    /// Op 4: this many per window are sent at once (Fluxer: 2 per second; a little margin).
    pub op4_per_window: usize,
    pub op4_window: Duration,
    /// Op 3: at most this many per window (Fluxer: 5 per 20 s; more are dropped silently).
    pub op3_per_window: usize,
    pub op3_window: Duration,
    /// Op 8: at most this many per window (Fluxer: 12 per 10 s; more are dropped silently).
    pub op8_per_window: usize,
    pub op8_window: Duration,
    /// How long a member search may wait for its answer.
    pub search_timeout: Duration,
    /// Opening the socket (TCP, TLS and the websocket handshake).
    pub connect_timeout: Duration,
    /// A frame that cannot be written in this long means the connection is dead.
    pub send_timeout: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        GatewayConfig {
            hello_timeout: Duration::from_secs(10),
            ready_timeout: Duration::from_secs(90),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            rate_limit_backoff: Duration::from_secs(30),
            healthy_after: Duration::from_secs(60),
            op4_per_window: 2,
            op4_window: Duration::from_millis(1100),
            op3_per_window: 5,
            op3_window: Duration::from_millis(20_500),
            op8_per_window: 10,
            op8_window: Duration::from_millis(10_500),
            search_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(30),
            send_timeout: Duration::from_secs(10),
        }
    }
}

type MemberReply = oneshot::Sender<Result<Vec<Member>, FluxerError>>;

pub(crate) enum Command {
    Voice(Value, oneshot::Sender<Result<(), FluxerError>>),
    /// Request Guild Members (op 8).
    Members(Search),
    Presence(Option<String>),
    Close(oneshot::Sender<()>),
}

/// A member search: one community, a name prefix, at most `limit` (Fluxer allows 100).
pub(crate) struct Search {
    pub guild: GuildId,
    pub query: String,
    pub limit: u32,
    pub reply: MemberReply,
}

/// The member search Fluxer is answering.
struct Searching {
    nonce: String,
    found: Vec<Member>,
    reply: MemberReply,
    until: tokio::time::Instant,
}

fn not_connected(why: &str) -> FluxerError {
    FluxerError::new(ErrorKind::NotConnected, why)
}

/// The handle the client keeps.
#[derive(Debug, Clone)]
pub(crate) struct Gateway {
    pub tx: mpsc::UnboundedSender<Command>,
}

/// Rolling-window pacing.
struct Pace {
    sent: VecDeque<Instant>,
    n: usize,
    window: Duration,
}

impl Pace {
    fn new(n: usize, window: Duration) -> Pace {
        Pace {
            sent: VecDeque::new(),
            n: n.max(1),
            window,
        }
    }

    /// When the next send may go (`None` = now).
    fn next(&mut self) -> Option<Instant> {
        let now = Instant::now();
        while self.sent.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            self.sent.pop_front();
        }
        if self.sent.len() < self.n {
            None
        } else {
            self.sent.front().map(|t| *t + self.window)
        }
    }

    fn record(&mut self) {
        self.sent.push_back(Instant::now());
    }
}

/// What the gateway carries across connections.
struct Session {
    id: Option<String>,
    seq: Option<u64>,
    presence: Option<String>,
    presence_dirty: bool,
}

/// The pacing of what Fluxer limits, kept across connections.
struct Paces {
    op3: Pace,
    op4: Pace,
    op8: Pace,
}

enum End {
    /// Reconnect; `code` is the close code (if any).
    Reconnect(Option<u16>),
    Fatal(Fatal),
    Closed,
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Where and how the gateway connects.
pub(crate) struct Target {
    pub url: Url,
    pub tls: Connector,
    pub token: SecretString,
    pub identity: BotIdentity,
}

pub(crate) fn spawn(
    target: Target,
    cfg: GatewayConfig,
    events: mpsc::UnboundedSender<GatewayEvent>,
    connected: watch::Sender<bool>,
) -> Gateway {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(run(target, cfg, events, connected, rx));
    Gateway { tx }
}

/// Runs `work` while the gateway is not connected, answering commands meanwhile: voice updates and member searches
/// fail at once (their callers try again), presence is remembered. `None` when the client closed the gateway.
async fn offline<F: std::future::Future>(
    rx: &mut mpsc::UnboundedReceiver<Command>,
    session: &mut Session,
    work: F,
) -> Option<F::Output> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return Some(out),
            cmd = rx.recv() => match cmd {
                None => return None,
                Some(Command::Close(done)) => {
                    let _ = done.send(());
                    return None;
                }
                Some(Command::Presence(p)) => {
                    session.presence = p;
                    session.presence_dirty = true;
                }
                Some(Command::Voice(_, reply)) => {
                    let _ = reply.send(Err(not_connected("the gateway is reconnecting")));
                }
                Some(Command::Members(s)) => {
                    let _ = s.reply.send(Err(not_connected("the gateway is reconnecting")));
                }
            },
        }
    }
}

fn backoff(cfg: &GatewayConfig, failures: u32) -> Duration {
    let base = cfg
        .backoff_base
        .saturating_mul(1 << failures.saturating_sub(1).min(10))
        .min(cfg.backoff_max);
    base.mul_f64(0.75 + fastrand::f64() * 0.5)
}

async fn run(
    target: Target,
    cfg: GatewayConfig,
    events: mpsc::UnboundedSender<GatewayEvent>,
    connected: watch::Sender<bool>,
    mut rx: mpsc::UnboundedReceiver<Command>,
) {
    let mut session = Session {
        id: None,
        seq: None,
        presence: None,
        presence_dirty: false,
    };
    let mut failures = 0u32;
    let mut paces = Paces {
        op3: Pace::new(cfg.op3_per_window, cfg.op3_window),
        op4: Pace::new(cfg.op4_per_window, cfg.op4_window),
        op8: Pace::new(cfg.op8_per_window, cfg.op8_window),
    };
    loop {
        let started = Instant::now();
        let connect = tokio::time::timeout(
            cfg.connect_timeout,
            tokio_tungstenite::connect_async_tls_with_config(
                target.url.as_str(),
                None,
                false,
                Some(target.tls.clone()),
            ),
        );
        let end = match offline(&mut rx, &mut session, connect).await {
            None => return,
            Some(Ok(Ok((ws, _)))) => {
                let mut c = Conn {
                    ws,
                    cfg: &cfg,
                    token: &target.token,
                    identity: &target.identity,
                    events: &events,
                    connected: &connected,
                };
                c.run(&mut session, &mut rx, &mut paces).await
            }
            Some(Ok(Err(e))) => {
                tracing::warn!(error = %e, "could not connect to the Fluxer gateway");
                End::Reconnect(None)
            }
            Some(Err(_)) => {
                tracing::warn!(
                    "connecting to the Fluxer gateway took longer than {:?}",
                    cfg.connect_timeout
                );
                End::Reconnect(None)
            }
        };
        connected.send_replace(false);
        let code = match end {
            End::Closed => return,
            End::Fatal(f) => {
                let _ = events.send(GatewayEvent::Stopped(f));
                // Answer what is still queued, then stop.
                rx.close();
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        Command::Voice(_, reply) => {
                            let _ = reply.send(Err(not_connected("the gateway stopped")));
                        }
                        Command::Members(s) => {
                            let _ = s.reply.send(Err(not_connected("the gateway stopped")));
                        }
                        Command::Close(done) => {
                            let _ = done.send(());
                        }
                        Command::Presence(_) => {}
                    }
                }
                return;
            }
            End::Reconnect(code) => code,
        };
        failures = if started.elapsed() >= cfg.healthy_after {
            1
        } else {
            failures + 1
        };
        let mut delay = backoff(&cfg, failures);
        match code {
            Some(4008) => delay = delay.max(cfg.rate_limit_backoff),
            Some(4003 | 4007) => {
                session.id = None;
                session.seq = None;
            }
            _ => {}
        }
        let resuming = session.id.is_some();
        let _ = events.send(GatewayEvent::Down { code, resuming });
        tracing::info!(?code, ?delay, resuming, "gateway down; reconnecting");
        if offline(&mut rx, &mut session, tokio::time::sleep(delay))
            .await
            .is_none()
        {
            return;
        }
    }
}

struct Conn<'a> {
    ws: Ws,
    cfg: &'a GatewayConfig,
    token: &'a SecretString,
    identity: &'a BotIdentity,
    events: &'a mpsc::UnboundedSender<GatewayEvent>,
    connected: &'a watch::Sender<bool>,
}

fn close_code(frame: Option<&CloseFrame>) -> Option<u16> {
    frame.map(|f| u16::from(f.code))
}

impl Conn<'_> {
    /// Writes a frame; `false` when the connection is dead (an error, or no progress within the send timeout).
    async fn send(&mut self, v: &Value) -> bool {
        tokio::time::timeout(self.cfg.send_timeout, self.ws.send(Message::text(v.to_string())))
            .await
            .is_ok_and(|r| r.is_ok())
    }

    /// Closes the socket (as far as the peer lets within the send timeout).
    async fn close(&mut self, code: CloseCode, reason: &'static str) {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        let _ = tokio::time::timeout(self.cfg.send_timeout, self.ws.close(Some(frame))).await;
    }

    async fn identify(&mut self) -> bool {
        let os = std::env::consts::OS;
        let v = json!({"op": 2, "d": {
            "token": self.token.expose_secret(),
            "properties": {"os": os, "browser": "profanity-watch-bot", "device": "bot"},
            "ignored_events": IGNORED_EVENTS,
        }});
        self.send(&v).await
    }

    /// Waits for the next frame as JSON; `Err(code)` when the socket closed.
    async fn recv(&mut self) -> Result<Value, Option<u16>> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<Value>(t.as_str()) {
                    Ok(v) => return Ok(v),
                    Err(e) => tracing::warn!(error = %e, "an unreadable gateway frame was skipped"),
                },
                Some(Ok(Message::Close(frame))) => return Err(close_code(frame.as_ref())),
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "gateway connection error");
                    return Err(None);
                }
                None => return Err(None),
            }
        }
    }

    async fn run(
        &mut self,
        session: &mut Session,
        rx: &mut mpsc::UnboundedReceiver<Command>,
        paces: &mut Paces,
    ) -> End {
        let hello = match tokio::time::timeout(self.cfg.hello_timeout, self.recv()).await {
            Ok(Ok(v)) if v.get("op").and_then(Value::as_u64) == Some(10) => v,
            Ok(Err(code)) => return self.closed(code, false),
            _ => {
                tracing::warn!("no Hello from the gateway");
                return End::Reconnect(None);
            }
        };
        let interval = Duration::from_millis(
            hello
                .pointer("/d/heartbeat_interval")
                .and_then(Value::as_u64)
                .unwrap_or(41_250),
        );
        let resuming = session.id.is_some();
        let sent = if let (Some(id), true) = (&session.id, resuming) {
            let v = json!({"op": 6, "d": {"token": self.token.expose_secret(), "session_id": id, "seq": session.seq.unwrap_or(0)}});
            self.send(&v).await
        } else {
            self.identify().await
        };
        if !sent {
            return End::Reconnect(None);
        }
        let mut acked = true;
        let mut beat = tokio::time::Instant::now() + interval.mul_f64(fastrand::f64());
        let mut ready = false;
        let ready_deadline = tokio::time::Instant::now() + self.cfg.ready_timeout;
        // Queued work dropped with this connection answers its callers that the connection dropped.
        let mut voice_queue: VecDeque<(Value, oneshot::Sender<Result<(), FluxerError>>)> = VecDeque::new();
        // Member searches go one at a time (Fluxer replaces a waiting search with a newer one); a newer search of the
        // same community replaces a queued one (someone typing a name).
        let mut searches: VecDeque<Search> = VecDeque::new();
        let mut searching: Option<Searching> = None;
        let mut nonces = 0u64;
        // READY's users: the communities that follow name their members by id only.
        let mut ready_users: std::collections::HashMap<UserId, pb_fluxer_api::User> = std::collections::HashMap::new();
        loop {
            // The next moment something paced may go out.
            let paced = {
                let v = (ready && !voice_queue.is_empty()).then(|| paces.op4.next()).flatten();
                let p = (ready && session.presence_dirty).then(|| paces.op3.next()).flatten();
                let s = (ready && searching.is_none() && !searches.is_empty())
                    .then(|| paces.op8.next())
                    .flatten();
                [v, p, s]
                    .into_iter()
                    .flatten()
                    .min()
                    .map(tokio::time::Instant::from_std)
            };
            if ready {
                if !voice_queue.is_empty()
                    && paces.op4.next().is_none()
                    && let Some((v, reply)) = voice_queue.pop_front()
                {
                    paces.op4.record();
                    let ok = self.send(&json!({"op": 4, "d": v})).await;
                    let _ = reply.send(if ok {
                        Ok(())
                    } else {
                        Err(not_connected("the gateway connection dropped"))
                    });
                    continue;
                }
                if searching.is_none()
                    && paces.op8.next().is_none()
                    && let Some(s) = searches.pop_front()
                {
                    paces.op8.record();
                    nonces += 1;
                    let nonce = format!("pb{nonces}");
                    let d = json!({"guild_id": s.guild.to_string(), "query": s.query, "limit": s.limit.min(100), "nonce": nonce});
                    if !self.send(&json!({"op": 8, "d": d})).await {
                        let _ = s.reply.send(Err(not_connected("the gateway connection dropped")));
                        return End::Reconnect(None);
                    }
                    searching = Some(Searching {
                        nonce,
                        found: Vec::new(),
                        reply: s.reply,
                        until: tokio::time::Instant::now() + self.cfg.search_timeout,
                    });
                    continue;
                }
                if session.presence_dirty && paces.op3.next().is_none() {
                    paces.op3.record();
                    session.presence_dirty = false;
                    let custom = session
                        .presence
                        .as_ref()
                        .map(|t| json!({"text": t.chars().take(128).collect::<String>()}));
                    let v = json!({"op": 3, "d": {"status": "online", "afk": false, "mobile": false, "custom_status": custom}});
                    if !self.send(&v).await {
                        return End::Reconnect(None);
                    }
                    continue;
                }
            }
            let wake = [paced, searching.as_ref().map(|s| s.until)]
                .into_iter()
                .flatten()
                .fold(beat, std::cmp::min);
            let deadline = if ready { None } else { Some(ready_deadline) };
            tokio::select! {
                frame = self.recv() => {
                    let v = match frame {
                        Ok(v) => v,
                        Err(code) => return self.closed(code, resuming),
                    };
                    match v.get("op").and_then(Value::as_u64) {
                        Some(0) => {
                            let t = v.get("t").and_then(Value::as_str).unwrap_or_default().to_owned();
                            let d = v.get("d").cloned().unwrap_or(Value::Null);
                            if t == "READY" {
                                session.id = d.get("session_id").and_then(Value::as_str).map(str::to_owned);
                                ready_users = d.get("users").and_then(Value::as_array).into_iter().flatten()
                                    .filter_map(wire::user).map(|u| (u.id, u)).collect();
                                ready = true;
                                session.presence_dirty |= session.presence.is_some();
                                self.connected.send_replace(true);
                                let guilds = d.get("guilds").and_then(Value::as_array).into_iter().flatten()
                                    .filter_map(|g| wire::id(g.get("id")).map(GuildId)).collect();
                                let mut bot = self.identity.clone();
                                if let Some(u) = d.get("user").and_then(wire::user) {
                                    bot.user = u;
                                }
                                let _ = self.events.send(GatewayEvent::Ready { bot, session: session.id.clone().unwrap_or_default(), guilds });
                            } else if t == "RESUMED" {
                                ready = true;
                                self.connected.send_replace(true);
                                let _ = self.events.send(GatewayEvent::Resumed);
                            } else {
                                if t == "GUILD_MEMBERS_CHUNK"
                                    && let Some(s) = searching.as_mut()
                                    && d.get("nonce").and_then(Value::as_str) == Some(s.nonce.as_str())
                                {
                                    s.found.extend(d.get("members").and_then(Value::as_array).into_iter().flatten().filter_map(wire::member));
                                    let index = d.get("chunk_index").and_then(Value::as_u64).unwrap_or(0);
                                    let count = d.get("chunk_count").and_then(Value::as_u64).unwrap_or(1);
                                    if index + 1 >= count
                                        && let Some(s) = searching.take()
                                    {
                                        let _ = s.reply.send(Ok(s.found));
                                    }
                                }
                                for mut ev in dispatch(&t, &d) {
                                    if let GatewayEvent::GuildAvailable(g) = &mut ev {
                                        for m in g.members.iter_mut().filter(|m| m.user.is_none()) {
                                            m.user = ready_users.get(&m.id).cloned();
                                        }
                                    }
                                    let _ = self.events.send(ev);
                                }
                            }
                            // Only a fully handled dispatch counts (the heartbeat's sequence trims the replay buffer).
                            if let Some(s) = v.get("s").and_then(Value::as_u64) {
                                session.seq = Some(s);
                            }
                        }
                        Some(1) => {
                            if !self.send(&json!({"op": 1, "d": session.seq})).await {
                                return End::Reconnect(None);
                            }
                        }
                        Some(11) => acked = true,
                        Some(7) => {
                            tracing::info!("the gateway asked to reconnect (op 7)");
                            self.close(CloseCode::from(4000), "reconnect").await;
                            return End::Reconnect(Some(4000));
                        }
                        Some(9) => {
                            // Never resumable; identify again on this socket.
                            tracing::info!("the gateway invalidated the session (op 9); identifying again");
                            session.id = None;
                            session.seq = None;
                            ready = false;
                            self.connected.send_replace(false);
                            if !self.identify().await {
                                return End::Reconnect(None);
                            }
                        }
                        other => tracing::debug!(?other, "an unknown gateway opcode was ignored"),
                    }
                }
                cmd = rx.recv() => match cmd {
                    None => return End::Closed,
                    Some(Command::Close(done)) => {
                        self.close(CloseCode::Normal, "bye").await;
                        let _ = done.send(());
                        return End::Closed;
                    }
                    Some(Command::Presence(p)) => {
                        session.presence = p;
                        session.presence_dirty = true;
                    }
                    Some(Command::Voice(v, reply)) => {
                        if ready {
                            voice_queue.push_back((v, reply));
                        } else {
                            let _ = reply.send(Err(not_connected("the gateway is not logged in yet")));
                        }
                    }
                    Some(Command::Members(s)) => {
                        if !ready {
                            let _ = s.reply.send(Err(not_connected("the gateway is not logged in yet")));
                        } else {
                            if let Some(i) = searches.iter().position(|q| q.guild == s.guild)
                                && let Some(old) = searches.remove(i)
                            {
                                let _ = old.reply.send(Err(FluxerError::new(ErrorKind::Superseded, "a newer search replaced this one")));
                            }
                            searches.push_back(s);
                        }
                    }
                },
                () = tokio::time::sleep_until(wake) => {
                    if searching.as_ref().is_some_and(|s| tokio::time::Instant::now() >= s.until)
                        && let Some(s) = searching.take()
                    {
                        let _ = s.reply.send(Err(FluxerError::new(ErrorKind::Network, "Fluxer did not answer the member search")));
                    }
                    if tokio::time::Instant::now() >= beat {
                        if !acked {
                            tracing::warn!("a heartbeat was not acknowledged; reconnecting");
                            return End::Reconnect(None);
                        }
                        acked = false;
                        beat = tokio::time::Instant::now() + interval;
                        if !self.send(&json!({"op": 1, "d": session.seq})).await {
                            return End::Reconnect(None);
                        }
                    }
                }
                () = async { if let Some(d) = deadline { tokio::time::sleep_until(d).await } else { std::future::pending().await } } => {
                    tracing::warn!("no READY within {:?}; reconnecting", self.cfg.ready_timeout);
                    return End::Reconnect(None);
                }
            }
        }
    }

    fn closed(&self, code: Option<u16>, resuming: bool) -> End {
        match code {
            Some(4004) if !resuming => End::Fatal(Fatal::TokenRejected),
            Some(4010) => End::Fatal(Fatal::Protocol {
                code: 4010,
                reason: "invalid shard".into(),
            }),
            Some(4011) => End::Fatal(Fatal::ShardingRequired),
            Some(4012) => End::Fatal(Fatal::Protocol {
                code: 4012,
                reason: "invalid API version".into(),
            }),
            // 4004 on Resume: the token does not own the session → identify.
            Some(4004) => End::Reconnect(Some(4007)),
            other => End::Reconnect(other),
        }
    }
}

/// A dispatch → events.
pub(crate) fn dispatch(t: &str, d: &Value) -> Vec<GatewayEvent> {
    let guild_of = |v: &Value| wire::id(v.get("guild_id")).map(GuildId);
    let one = |e: Option<GatewayEvent>| e.into_iter().collect::<Vec<_>>();
    match t {
        "GUILD_CREATE" => {
            if d.get("unavailable").and_then(Value::as_bool) == Some(true) {
                one(wire::id(d.get("id")).map(|g| GatewayEvent::GuildUnavailable(GuildId(g))))
            } else {
                one(wire::guild(d).map(|g| GatewayEvent::GuildAvailable(Box::new(g))))
            }
        }
        "GUILD_DELETE" => one(wire::id(d.get("id")).map(GuildId).map(|g| {
            if d.get("unavailable").and_then(Value::as_bool) == Some(true) {
                GatewayEvent::GuildUnavailable(g)
            } else {
                GatewayEvent::GuildRemoved(g)
            }
        })),
        "GUILD_UPDATE" => one(wire::id(d.get("guild_id")).or_else(|| wire::id(d.get("id"))).map(|g| {
            let (name, icon, owner) = wire::guild_props(d);
            GatewayEvent::GuildUpdated {
                guild: GuildId(g),
                name,
                icon,
                owner,
            }
        })),
        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            one(guild_of(d)
                .zip(d.get("role").and_then(wire::role))
                .map(|(guild, r)| GatewayEvent::RolesChanged {
                    guild,
                    upsert: vec![r],
                    removed: vec![],
                }))
        }
        "GUILD_ROLE_UPDATE_BULK" => one(guild_of(d).map(|guild| GatewayEvent::RolesChanged {
            guild,
            upsert: d
                .get("roles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(wire::role)
                .collect(),
            removed: vec![],
        })),
        "GUILD_ROLE_DELETE" => {
            one(guild_of(d)
                .zip(wire::id(d.get("role_id")))
                .map(|(guild, r)| GatewayEvent::RolesChanged {
                    guild,
                    upsert: vec![],
                    removed: vec![RoleId(r)],
                }))
        }
        "CHANNEL_CREATE" | "CHANNEL_UPDATE" => {
            one(guild_of(d)
                .zip(wire::channel(d))
                .map(|(guild, c)| GatewayEvent::ChannelsChanged {
                    guild,
                    upsert: vec![c],
                    removed: vec![],
                }))
        }
        "CHANNEL_UPDATE_BULK" => one(guild_of(d).map(|guild| GatewayEvent::ChannelsChanged {
            guild,
            upsert: d
                .get("channels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(wire::channel)
                .collect(),
            removed: vec![],
        })),
        "CHANNEL_DELETE" => {
            one(guild_of(d)
                .zip(wire::id(d.get("id")))
                .map(|(guild, c)| GatewayEvent::ChannelsChanged {
                    guild,
                    upsert: vec![],
                    removed: vec![pb_domain::ChannelId(c)],
                }))
        }
        "GUILD_MEMBER_ADD" | "GUILD_MEMBER_UPDATE" => {
            one(guild_of(d)
                .zip(wire::member(d))
                .map(|(guild, m)| GatewayEvent::MemberUpdated {
                    guild,
                    member: Box::new(m),
                }))
        }
        // Answers to member searches: every member is news for the directory too.
        "GUILD_MEMBERS_CHUNK" => match guild_of(d) {
            Some(guild) => d
                .get("members")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(wire::member)
                .map(|m| GatewayEvent::MemberUpdated {
                    guild,
                    member: Box::new(m),
                })
                .collect(),
            None => Vec::new(),
        },
        "GUILD_MEMBER_REMOVE" => one(guild_of(d)
            .zip(wire::id(d.pointer("/user/id")))
            .map(|(guild, u)| GatewayEvent::MemberRemoved { guild, user: UserId(u) })),
        "VOICE_STATE_UPDATE" => one(wire::voice_state(d, None).map(|vs| GatewayEvent::VoiceState {
            state: Box::new(vs),
            member: d.get("member").and_then(wire::member).map(Box::new),
        })),
        "VOICE_SERVER_UPDATE" => one(wire::voice_grant(d).map(|g| GatewayEvent::VoiceServer(Box::new(g)))),
        "MESSAGE_CREATE" => one(wire::message(d).map(|m| GatewayEvent::Message(Box::new(m)))),
        _ => Vec::new(),
    }
}
