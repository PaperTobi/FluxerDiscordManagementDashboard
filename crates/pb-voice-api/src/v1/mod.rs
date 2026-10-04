//! Version 1 of the voice interface.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use pb_domain::{ConnectionId, UserId};
use pb_fluxer_api::VoiceGrant;
use tokio::sync::mpsc;

/// The transport hands the bot audio at [`LISTEN_RATE`] and takes it at [`PLAY_RATE`].
pub use pb_domain::{LISTEN_RATE, PLAY_RATE};

/// A participant in the room: the transport's name for them and, when the transport can tell, the Fluxer account and
/// voice connection behind it (how a platform names its participants is the transport's business).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identity {
    pub name: String,
    pub person: Option<(UserId, ConnectionId)>,
}

impl Identity {
    /// The Fluxer account behind this participant.
    pub fn user(&self) -> Option<UserId> {
        self.person.as_ref().map(|(u, _)| *u)
    }
}

/// What a remote audio track carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSource {
    Microphone,
    ScreenShareAudio,
    Unknown,
}

/// A remote track, addressed by its participant and the server's track id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackKey {
    pub participant: Identity,
    pub track_sid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAudioTrack {
    pub key: TrackKey,
    pub source: TrackSource,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub identity: Identity,
    pub audio: Vec<RemoteAudioTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomEvent {
    ParticipantJoined(Participant),
    ParticipantLeft(Identity),
    TrackPublished(RemoteAudioTrack),
    TrackUnpublished(TrackKey),
    TrackMuted {
        key: TrackKey,
        muted: bool,
    },
    Reconnecting,
    Reconnected,
    /// The room is gone; no more events follow.
    Disconnected {
        reason: String,
    },
}

/// A piece of received audio: mono, signed 16-bit, [`LISTEN_RATE`].
#[derive(Debug, Clone)]
pub struct PcmChunk {
    pub samples: Vec<i16>,
    /// When the transport handed it over.
    pub received: Instant,
}

/// Received audio of one subscribed track. Ends when the track is unsubscribed or the room closes.
pub type AudioIn = mpsc::UnboundedReceiver<PcmChunk>;

/// Who hears the bot's voice track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudienceSet {
    All,
    Only(Vec<Identity>),
}

#[derive(Debug, Clone)]
pub struct ConnectOpts {
    /// Give up joining after this long.
    pub timeout: Duration,
}

impl Default for ConnectOpts {
    fn default() -> Self {
        ConnectOpts {
            timeout: Duration::from_secs(12),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("this transport cannot use that kind of voice grant")]
    UnsupportedGrant,
    #[error("joining timed out")]
    Timeout,
    #[error("could not join: {0}")]
    Connect(String),
    #[error("the bot may not speak in this channel")]
    NoSpeakPermission,
    #[error("no such track: {0:?}")]
    NoSuchTrack(TrackKey),
    #[error("the room is closed")]
    Closed,
    #[error("{0}")]
    Other(String),
}

/// Joins voice rooms. Automatic subscription is always off: the bot receives only what it subscribes to.
#[async_trait]
pub trait VoiceTransport: Send + Sync + 'static {
    async fn connect(
        &self,
        grant: &VoiceGrant,
        opts: ConnectOpts,
    ) -> Result<(Box<dyn VoiceRoom>, mpsc::UnboundedReceiver<RoomEvent>), TransportError>;
}

/// One joined room.
#[async_trait]
pub trait VoiceRoom: Send + Sync {
    /// Everyone else in the room right now, with their audio tracks.
    fn participants(&self) -> Vec<Participant>;
    /// Start receiving one remote audio track.
    async fn subscribe(&self, track: &TrackKey) -> Result<AudioIn, TransportError>;
    async fn unsubscribe(&self, track: &TrackKey) -> Result<(), TransportError>;
    /// Publish the bot's single voice track (as a microphone).
    async fn publish_voice(&self) -> Result<Box<dyn AudioOut>, TransportError>;
    /// Decide who may hear the bot's voice track.
    async fn set_audience(&self, audience: &AudienceSet) -> Result<(), TransportError>;
    async fn close(&self);
}

/// The bot's voice track.
#[async_trait]
pub trait AudioOut: Send {
    /// Play mono 16-bit [`PLAY_RATE`] audio; returns once it has been played out. Dropping the future stops it
    /// (with at most the transport's buffer still heard).
    async fn play(&mut self, pcm48: &[i16]) -> Result<(), TransportError>;
}
