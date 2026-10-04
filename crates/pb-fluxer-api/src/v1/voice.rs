use pb_domain::{ChannelId, ConnectionId, GuildId};
use secrecy::SecretString;
use url::Url;

/// What Fluxer hands out after a voice-state update: where and how the bot may join a voice channel.
///
/// Fluxer currently runs voice on LiveKit. Its roadmap replaces LiveKit with its own voice system; that system will be
/// a new variant here with its own transport crate, which is why the enum is non-exhaustive.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VoiceGrant {
    LiveKit {
        guild: GuildId,
        channel: ChannelId,
        connection: ConnectionId,
        /// `wss://…` LiveKit server address.
        endpoint: Url,
        /// LiveKit access token (a JWT) for this connection.
        token: SecretString,
        /// Present when the channel uses end-to-end encryption.
        e2ee_key: Option<SecretString>,
    },
}

impl VoiceGrant {
    pub fn guild(&self) -> GuildId {
        match self {
            VoiceGrant::LiveKit { guild, .. } => *guild,
        }
    }

    pub fn channel(&self) -> ChannelId {
        match self {
            VoiceGrant::LiveKit { channel, .. } => *channel,
        }
    }

    pub fn connection(&self) -> &ConnectionId {
        match self {
            VoiceGrant::LiveKit { connection, .. } => connection,
        }
    }
}
