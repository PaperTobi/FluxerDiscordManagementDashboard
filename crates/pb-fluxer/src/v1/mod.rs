//! Version 1.

mod gateway;
mod pace;
mod rest;
mod wire;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use pb_domain::{ChannelId, GuildId, MessageId, UserId};
use pb_fluxer_api::{
    Application, BotIdentity, Destination, Endpoints, ErrorKind, Fluxer, FluxerCtl, FluxerError, GatewayEvent,
    LoginError, Member, MemberPatch, MessageRef, OAuthClient, OAuthUser, OutgoingMessage, VoiceStateOp,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

pub use gateway::GatewayConfig;
use rest::Rest;

/// Reads `/.well-known/fluxer`.
/// Looks up an instance's endpoints at `{instance}/.well-known/fluxer`, and for an address with a path (an API address
/// like `https://example.com/api`) at the server's root when it is not there.
async fn discover(instance: &Url) -> Result<Endpoints, LoginError> {
    match discover_at(instance).await {
        Err(LoginError::BadInstance(why)) if instance.path() != "/" => {
            let mut root = instance.clone();
            root.set_path("/");
            discover_at(&root).await.map_err(|_| LoginError::BadInstance(why))
        }
        found => found,
    }
}

async fn discover_at(instance: &Url) -> Result<Endpoints, LoginError> {
    let url = format!("{}/.well-known/fluxer", instance.as_str().trim_end_matches('/'));
    let http = rest::http().map_err(|e| LoginError::Unreachable(e.to_string()))?;
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| LoginError::Unreachable(format!("{url}: {e}")))?;
    let status = resp.status().as_u16();
    if status == 429 || status >= 500 {
        return Err(LoginError::Unreachable(format!("{url} answered HTTP {status}")));
    }
    if status != 200 {
        return Err(LoginError::BadInstance(format!("{url} answered HTTP {status}")));
    }
    let doc: Value = resp.json().await.map_err(|_| {
        LoginError::BadInstance(format!(
            "{url} did not return JSON (a web page?); use the API address, like https://api.fluxer.app"
        ))
    })?;
    parse_discovery(&doc)
        .ok_or_else(|| LoginError::BadInstance(format!("{url} has no endpoints.api_public / endpoints.gateway")))
}

/// The endpoints of a discovery document.
fn parse_discovery(doc: &Value) -> Option<Endpoints> {
    let ep = doc.get("endpoints")?;
    let url = |k: &str| {
        ep.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .and_then(|s| Url::parse(s).ok())
    };
    Some(Endpoints {
        api: url("api_public").or_else(|| url("api"))?,
        gateway: url("gateway")?,
        media: url("media"),
        voice_enabled: doc
            .pointer("/features/voice_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

/// The real Fluxer.
#[derive(Debug, Clone, Default)]
pub struct FluxerClient {
    pub gateway: GatewayConfig,
}

#[async_trait]
impl Fluxer for FluxerClient {
    async fn discover(&self, instance: &Url) -> Result<Endpoints, LoginError> {
        discover(instance).await
    }

    async fn login(
        &self,
        ep: &Endpoints,
        token: &SecretString,
    ) -> Result<(Arc<dyn FluxerCtl>, mpsc::UnboundedReceiver<GatewayEvent>), LoginError> {
        let rest = Rest::new(ep.clone(), Some(token.clone())).map_err(|e| LoginError::Unreachable(e.to_string()))?;
        let check = |e: FluxerError| match e.kind {
            ErrorKind::Unauthorized => LoginError::TokenRejected,
            ErrorKind::Network | ErrorKind::Server => LoginError::Unreachable(e.to_string()),
            _ => LoginError::Refused(e.to_string()),
        };
        let app = rest.application().await.map_err(check)?;
        let user = rest.me().await.map_err(check)?;
        let identity = BotIdentity {
            user,
            application: app.id,
            owner: app.owner.as_ref().map(|o| o.id),
        };
        let tls = pb_tls::client_config().map_err(|e| LoginError::Unreachable(e.to_string()))?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (connected_tx, connected_rx) = watch::channel(false);
        let gw = gateway::spawn(
            gateway::Target {
                url: ep.gateway_url(),
                tls: tokio_tungstenite::Connector::Rustls(tls),
                token: token.clone(),
                identity: identity.clone(),
            },
            self.gateway.clone(),
            events_tx,
            connected_tx,
        );
        let ctl = Ctl {
            rest,
            gw,
            identity,
            dms: Mutex::new(HashMap::new()),
            connected: connected_rx,
        };
        Ok((Arc::new(ctl), events_rx))
    }

    async fn oauth_user(
        &self,
        ep: &Endpoints,
        client: &OAuthClient,
        code: &str,
        verifier: &str,
    ) -> Result<OAuthUser, FluxerError> {
        let rest = Rest::new(ep.clone(), None)?;
        let token = rest
            .oauth_token(
                client.client_id,
                client.client_secret.expose_secret(),
                code,
                verifier,
                client.redirect_uri.as_str(),
            )
            .await?;
        let v = rest.oauth_userinfo(&token).await?;
        wire::oauth_user(&v).ok_or_else(|| FluxerError::new(ErrorKind::Server, "userinfo has no id"))
    }

    async fn client_secret_ok(
        &self,
        ep: &Endpoints,
        client_id: u64,
        secret: &SecretString,
    ) -> Result<bool, FluxerError> {
        let rest = Rest::new(ep.clone(), None)?;
        let code = format!("pb-check-{}", fastrand::u64(..));
        match rest
            .oauth_token(
                client_id,
                secret.expose_secret(),
                &code,
                "pb-check",
                "https://pb-check.invalid/",
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if e.is_code("invalid_client") => Ok(false),
            Err(e) if e.is_code("invalid_grant") => Ok(true),
            Err(e) => Err(e),
        }
    }
}

/// A logged-in bot.
struct Ctl {
    rest: Rest,
    gw: gateway::Gateway,
    identity: BotIdentity,
    dms: Mutex<HashMap<UserId, ChannelId>>,
    connected: watch::Receiver<bool>,
}

impl Ctl {
    /// The direct-message channels opened so far (a poisoned lock is still a good cache).
    fn dms(&self) -> std::sync::MutexGuard<'_, HashMap<UserId, ChannelId>> {
        self.dms.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Ctl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctl")
            .field("bot", &self.identity.user.id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl FluxerCtl for Ctl {
    fn me(&self) -> BotIdentity {
        self.identity.clone()
    }

    fn endpoints(&self) -> &Endpoints {
        self.rest.endpoints()
    }

    async fn voice_state(&self, op: VoiceStateOp) -> Result<(), FluxerError> {
        if !*self.connected.borrow() {
            return Err(FluxerError::new(
                ErrorKind::NotConnected,
                "the gateway is not connected",
            ));
        }
        let mut d = json!({"self_mute": false, "self_deaf": false, "self_video": false, "self_stream": false});
        match op {
            VoiceStateOp::Join { guild, channel } => {
                d["guild_id"] = json!(guild.to_string());
                d["channel_id"] = json!(channel.to_string());
            }
            VoiceStateOp::Update {
                guild,
                channel,
                connection,
            } => {
                d["guild_id"] = json!(guild.to_string());
                d["channel_id"] = json!(channel.to_string());
                d["connection_id"] = json!(connection.0);
            }
            VoiceStateOp::Leave { guild, connection } => {
                d["guild_id"] = json!(guild.to_string());
                d["channel_id"] = Value::Null;
                d["connection_id"] = json!(connection.0);
            }
        }
        let (tx, rx) = oneshot::channel();
        self.gw
            .tx
            .send(gateway::Command::Voice(d, tx))
            .map_err(|_| FluxerError::new(ErrorKind::NotConnected, "the gateway stopped"))?;
        rx.await
            .map_err(|_| FluxerError::new(ErrorKind::NotConnected, "the gateway stopped"))?
    }

    fn presence(&self, text: Option<String>) {
        let _ = self.gw.tx.send(gateway::Command::Presence(text));
    }

    async fn send(&self, to: Destination, m: OutgoingMessage) -> Result<MessageId, FluxerError> {
        let channel = match to {
            Destination::Channel(c) => c,
            Destination::User(u) => {
                let known = self.dms().get(&u).copied();
                match known {
                    Some(c) => c,
                    None => {
                        let c = self.rest.open_dm(u).await?;
                        self.dms().insert(u, c);
                        c
                    }
                }
            }
        };
        let result = self.rest.create_message(channel, &m).await;
        if let (Destination::User(u), Err(e)) = (to, &result)
            && e.kind == ErrorKind::NotFound
        {
            // The direct-message channel is gone: open it again next time.
            self.dms().remove(&u);
        }
        result
    }

    async fn react(&self, m: MessageRef, emoji: &str) -> Result<(), FluxerError> {
        self.rest.react(m.channel, m.message, emoji).await
    }

    async fn patch_member(&self, guild: GuildId, user: UserId, patch: MemberPatch) -> Result<(), FluxerError> {
        self.rest.patch_member(guild, user, &patch).await
    }

    async fn search_members(&self, guild: GuildId, query: &str, limit: u32) -> Result<Vec<Member>, FluxerError> {
        let (reply, rx) = oneshot::channel();
        let cmd = gateway::Command::Members(gateway::Search {
            guild,
            query: query.to_owned(),
            limit,
            reply,
        });
        let dropped = || FluxerError::new(ErrorKind::NotConnected, "the gateway connection dropped");
        self.gw.tx.send(cmd).map_err(|_| dropped())?;
        // The gateway answers every search (found, superseded, timed out or dropped with the connection).
        rx.await.unwrap_or_else(|_| Err(dropped()))
    }

    async fn member(&self, guild: GuildId, user: UserId) -> Result<Option<Member>, FluxerError> {
        self.rest.member(guild, user).await
    }

    async fn application(&self) -> Result<Application, FluxerError> {
        self.rest.application().await
    }

    async fn close(&self) {
        let (tx, rx) = oneshot::channel();
        if self.gw.tx.send(gateway::Command::Close(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}
