//! Version 1 of the Fluxer interface: what the engine needs from Fluxer, independent of how it is reached.

mod endpoints;
mod error;
mod events;
mod model;
mod oauth;
mod ops;
pub mod perms;
mod voice;

use std::sync::Arc;

use async_trait::async_trait;
use pb_domain::{GuildId, MessageId, UserId};
use secrecy::SecretString;
use tokio::sync::mpsc;
use url::Url;

pub use endpoints::Endpoints;
pub use error::{ErrorKind, FluxerError, LoginError};
pub use events::{Fatal, GatewayEvent};
pub use model::{
    Application, BotIdentity, Channel, ChannelKind, Guild, IncomingMessage, Member, Overwrite, OverwriteKind, Role,
    User,
};
pub use oauth::{OAuthClient, OAuthUser, authorize_url};
pub use ops::{Attachment, Destination, MemberPatch, MessageRef, OutgoingMessage, VoiceStateOp};
pub use voice::VoiceGrant;

/// A way to Fluxer.
#[async_trait]
pub trait Fluxer: Send + Sync + 'static {
    /// Reads an instance's endpoints (`instance` is the API origin, e.g. `https://api.fluxer.app`).
    async fn discover(&self, instance: &Url) -> Result<Endpoints, LoginError>;

    /// Checks the token and connects the gateway. Events arrive on the receiver until the control handle is closed or
    /// the gateway stops for good ([`GatewayEvent::Stopped`]).
    async fn login(
        &self,
        ep: &Endpoints,
        token: &SecretString,
    ) -> Result<(Arc<dyn FluxerCtl>, mpsc::UnboundedReceiver<GatewayEvent>), LoginError>;

    /// Finishes a web login: exchanges the code and reads who logged in. A client secret Fluxer does not accept fails
    /// with the code `invalid_client`.
    async fn oauth_user(
        &self,
        ep: &Endpoints,
        client: &OAuthClient,
        code: &str,
        verifier: &str,
    ) -> Result<OAuthUser, FluxerError>;

    /// Whether Fluxer accepts `secret` as the client secret of application `client_id`. (Fluxer checks the client
    /// before the code, so exchanging a code that does not exist answers `invalid_client` for a wrong secret and
    /// `invalid_grant` for a right one.)
    async fn client_secret_ok(
        &self,
        ep: &Endpoints,
        client_id: u64,
        secret: &SecretString,
    ) -> Result<bool, FluxerError>;
}

/// A logged-in bot.
#[async_trait]
pub trait FluxerCtl: Send + Sync {
    fn me(&self) -> BotIdentity;

    fn endpoints(&self) -> &Endpoints;

    /// Sends a voice-state update (paced to what the gateway accepts at once). `NotConnected` while reconnecting.
    async fn voice_state(&self, op: VoiceStateOp) -> Result<(), FluxerError>;

    /// The bot's custom status (`None` clears it). The latest value wins; it is sent no faster than the gateway
    /// accepts (5 per 20 s) and again after a reconnect.
    fn presence(&self, text: Option<String>);

    /// Sends a message. Rate limits and outages are waited out (the message is never dropped); permanent refusals
    /// (no permission, a person who does not take direct messages) are errors.
    async fn send(&self, to: Destination, m: OutgoingMessage) -> Result<MessageId, FluxerError>;

    async fn react(&self, m: MessageRef, emoji: &str) -> Result<(), FluxerError>;

    async fn patch_member(&self, guild: GuildId, user: UserId, patch: MemberPatch) -> Result<(), FluxerError>;

    /// `None` when the person is not a member.
    async fn member(&self, guild: GuildId, user: UserId) -> Result<Option<Member>, FluxerError>;

    /// Members of a community whose name starts with `query` (at most `limit`; Fluxer answers up to 100), through
    /// the gateway (Request Guild Members). `NotConnected` while reconnecting.
    async fn search_members(&self, guild: GuildId, query: &str, limit: u32) -> Result<Vec<Member>, FluxerError>;

    async fn application(&self) -> Result<Application, FluxerError>;

    /// Leaves the gateway (voice connections end with the session).
    async fn close(&self);
}
