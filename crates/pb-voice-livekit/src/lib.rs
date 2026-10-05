//! [`VoiceTransport`] on the official LiveKit Rust SDK (`livekit`, which links Google's libwebrtc; see
//! docs/exceptions.toml). This is the only crate that knows LiveKit exists.
//!
//! Received audio is resampled to 16 kHz mono by libwebrtc and forwarded without a queue limit; the bot's voice is
//! published as one 48 kHz mono microphone track fed in 10 ms frames.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use livekit::RoomError;
use livekit::options::TrackPublishOptions;
use livekit::participant::ParticipantTrackPermission;
use livekit::prelude::{
    LocalAudioTrack, LocalTrack, ParticipantIdentity, RemoteParticipant, RemoteTrack, RemoteTrackPublication, Room,
    RoomEvent as LkEvent, RoomOptions, TrackKind, TrackSource as LkSource,
};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions};
use livekit::webrtc::prelude::AudioFrame;
use livekit_protocol::TrackSource as ProtoSource;
use livekit_protocol::request_response::Reason;
use pb_domain::ConnectionId;
use pb_fluxer_api::VoiceGrant;
use pb_voice_api::{
    AudienceSet, AudioIn, AudioOut, ConnectOpts, Identity, LISTEN_RATE, PLAY_RATE, Participant, PcmChunk,
    RemoteAudioTrack, RoomEvent, TrackKey, TrackSource, TransportError, VoiceRoom, VoiceTransport,
};
use secrecy::ExposeSecret;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

mod net;

pub use net::install as install_network;

/// How long to wait for the server to deliver a track after asking to subscribe.
const SUBSCRIBE_WAIT: Duration = Duration::from_secs(10);
/// Buffer inside libwebrtc's audio source.
const OUT_QUEUE_MS: u32 = 100;
const OUT_FRAME: usize = (PLAY_RATE / 100) as usize;

/// The LiveKit voice transport.
#[derive(Debug, Default, Clone, Copy)]
pub struct LiveKitTransport;

#[async_trait]
impl VoiceTransport for LiveKitTransport {
    async fn connect(
        &self,
        grant: &VoiceGrant,
        opts: ConnectOpts,
    ) -> Result<(Box<dyn VoiceRoom>, mpsc::UnboundedReceiver<RoomEvent>), TransportError> {
        let (url, token) = match grant {
            VoiceGrant::LiveKit { endpoint, token, .. } => {
                (endpoint.as_str().to_owned(), token.expose_secret().to_owned())
            }
            _ => return Err(TransportError::UnsupportedGrant),
        };
        net::install().map_err(TransportError::Connect)?;
        let mut options = RoomOptions::default();
        options.auto_subscribe = false;
        options.connect_timeout = opts.timeout;
        let (room, lk_events) = tokio::time::timeout(opts.timeout, Room::connect(&url, &token, options))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| TransportError::Connect(e.to_string()))?;
        let shared = Arc::new(Shared {
            room,
            waiting: Mutex::new(HashMap::new()),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(pump_events(Arc::clone(&shared), lk_events, tx));
        Ok((Box::new(LiveKitRoom { shared }), rx))
    }
}

/// Where a subscribed track is delivered (or why it will not be).
type Delivery = oneshot::Sender<Result<RemoteTrack, String>>;

struct Shared {
    room: Room,
    /// Subscriptions asked for and not yet delivered, by track sid.
    /// Who waits for a track to be delivered (several can ask for the same one).
    waiting: Mutex<HashMap<String, Vec<Delivery>>>,
}

struct LiveKitRoom {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for LiveKitRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveKitRoom").finish_non_exhaustive()
    }
}

fn identity(p: &RemoteParticipant) -> Identity {
    identity_named(p.identity().as_str())
}

/// A participant as Fluxer names them in its LiveKit rooms: `user_<user id>_<connection id>`.
pub fn identity_named(name: &str) -> Identity {
    let person = name.strip_prefix("user_").and_then(|rest| {
        let (uid, conn) = rest.split_once('_')?;
        (!conn.is_empty()).then_some(())?;
        Some((uid.parse().ok()?, ConnectionId(conn.to_owned())))
    });
    Identity {
        name: name.to_owned(),
        person,
    }
}

fn source(s: LkSource) -> TrackSource {
    match s {
        LkSource::Microphone => TrackSource::Microphone,
        LkSource::ScreenshareAudio => TrackSource::ScreenShareAudio,
        _ => TrackSource::Unknown,
    }
}

fn audio_track(participant: &RemoteParticipant, publication: &RemoteTrackPublication) -> Option<RemoteAudioTrack> {
    (publication.kind() == TrackKind::Audio).then(|| RemoteAudioTrack {
        key: TrackKey {
            participant: identity(participant),
            track_sid: publication.sid().to_string(),
        },
        source: source(publication.source()),
        muted: publication.is_muted(),
    })
}

fn participant(p: &RemoteParticipant) -> Participant {
    let audio = p
        .track_publications()
        .values()
        .filter_map(|publication| audio_track(p, publication))
        .collect();
    Participant {
        identity: identity(p),
        audio,
    }
}

async fn pump_events(
    shared: Arc<Shared>,
    mut lk: mpsc::UnboundedReceiver<LkEvent>,
    tx: mpsc::UnboundedSender<RoomEvent>,
) {
    while let Some(event) = lk.recv().await {
        let out = match event {
            LkEvent::ParticipantConnected(p) => Some(RoomEvent::ParticipantJoined(participant(&p))),
            LkEvent::ParticipantDisconnected(p) => Some(RoomEvent::ParticipantLeft(identity(&p))),
            LkEvent::TrackPublished {
                publication,
                participant,
            } => audio_track(&participant, &publication).map(RoomEvent::TrackPublished),
            LkEvent::TrackUnpublished {
                publication,
                participant,
            } => audio_track(&participant, &publication).map(|t| RoomEvent::TrackUnpublished(t.key)),
            LkEvent::TrackSubscribed { track, publication, .. } => {
                for waiter in lock(&shared.waiting)
                    .remove(&publication.sid().to_string())
                    .unwrap_or_default()
                {
                    let _ = waiter.send(Ok(track.clone()));
                }
                None
            }
            // Those waiting hear at once that it will not come (instead of after the wait).
            LkEvent::TrackSubscriptionFailed { track_sid, error, .. } => {
                warn!(track = %track_sid, %error, "a track could not be subscribed to");
                for waiter in lock(&shared.waiting).remove(&track_sid.to_string()).unwrap_or_default() {
                    let _ = waiter.send(Err(error.to_string()));
                }
                None
            }
            LkEvent::TrackMuted {
                participant,
                publication,
            }
            | LkEvent::TrackUnmuted {
                participant,
                publication,
            } => (publication.kind() == TrackKind::Audio).then(|| RoomEvent::TrackMuted {
                key: TrackKey {
                    participant: identity_named(participant.identity().as_str()),
                    track_sid: publication.sid().to_string(),
                },
                muted: publication.is_muted(),
            }),
            LkEvent::Reconnecting => Some(RoomEvent::Reconnecting),
            LkEvent::Reconnected => Some(RoomEvent::Reconnected),
            LkEvent::Disconnected { reason } => Some(RoomEvent::Disconnected {
                reason: format!("{reason:?}"),
            }),
            _ => None,
        };
        if let Some(out) = out {
            let last = matches!(out, RoomEvent::Disconnected { .. });
            if tx.send(out).is_err() || last {
                break;
            }
        }
    }
}

impl LiveKitRoom {
    fn publication(&self, key: &TrackKey) -> Result<RemoteTrackPublication, TransportError> {
        let participants = self.shared.room.remote_participants();
        let p = participants
            .get(&ParticipantIdentity::from(key.participant.name.clone()))
            .ok_or_else(|| TransportError::NoSuchTrack(key.clone()))?;
        p.track_publications()
            .into_values()
            .find(|publication| publication.sid().as_str() == key.track_sid && publication.kind() == TrackKind::Audio)
            .ok_or_else(|| TransportError::NoSuchTrack(key.clone()))
    }
}

#[async_trait]
impl VoiceRoom for LiveKitRoom {
    fn participants(&self) -> Vec<Participant> {
        self.shared
            .room
            .remote_participants()
            .values()
            .map(participant)
            .collect()
    }

    async fn subscribe(&self, key: &TrackKey) -> Result<AudioIn, TransportError> {
        let publication = self.publication(key)?;
        let track = match publication.track() {
            Some(track) => track,
            None => {
                let (tx, rx) = oneshot::channel();
                lock(&self.shared.waiting)
                    .entry(key.track_sid.clone())
                    .or_default()
                    .push(tx);
                publication.set_subscribed(true);
                match tokio::time::timeout(SUBSCRIBE_WAIT, rx).await {
                    Ok(Ok(Ok(track))) => track,
                    Ok(Ok(Err(e))) => return Err(TransportError::Other(format!("subscribing failed: {e}"))),
                    Ok(Err(_)) => return Err(TransportError::Closed),
                    Err(_) => {
                        lock(&self.shared.waiting).remove(&key.track_sid);
                        return Err(TransportError::Timeout);
                    }
                }
            }
        };
        let RemoteTrack::Audio(audio) = track else {
            return Err(TransportError::NoSuchTrack(key.clone()));
        };
        // Some(0) = no queue limit: libwebrtc's default drops the oldest audio once 10 frames are waiting.
        let options = NativeAudioStreamOptions {
            queue_size_frames: Some(0),
        };
        let mut stream = NativeAudioStream::with_options(audio.rtc_track(), LISTEN_RATE as i32, 1, options);
        let (tx, rx) = mpsc::unbounded_channel();
        let sid = key.track_sid.clone();
        tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                let chunk = PcmChunk {
                    samples: frame.data.into_owned(),
                    received: Instant::now(),
                };
                if tx.send(chunk).is_err() {
                    break;
                }
            }
            debug!(track = %sid, "audio stream ended");
            stream.close();
        });
        Ok(rx)
    }

    async fn unsubscribe(&self, key: &TrackKey) -> Result<(), TransportError> {
        self.publication(key)?.set_subscribed(false);
        Ok(())
    }

    async fn publish_voice(&self) -> Result<Box<dyn AudioOut>, TransportError> {
        let local = self.shared.room.local_participant();
        if let Some(permission) = local.permission() {
            let source_ok = permission.can_publish_sources.is_empty()
                || permission
                    .can_publish_sources
                    .contains(&(ProtoSource::Microphone as i32));
            if !permission.can_publish || !source_ok {
                return Err(TransportError::NoSpeakPermission);
            }
        }
        let source = NativeAudioSource::new(AudioSourceOptions::default(), PLAY_RATE, 1, OUT_QUEUE_MS);
        let track = LocalAudioTrack::create_audio_track("voice", RtcAudioSource::Native(source.clone()));
        let options = TrackPublishOptions {
            source: LkSource::Microphone,
            ..Default::default()
        };
        local
            .publish_track(LocalTrack::Audio(track), options)
            .await
            .map_err(|e| match e {
                RoomError::Request {
                    reason: Reason::NotAllowed,
                    ..
                } => TransportError::NoSpeakPermission,
                other => TransportError::Other(other.to_string()),
            })?;
        Ok(Box::new(LiveKitVoice {
            source,
            clock: PlayoutClock::default(),
        }))
    }

    async fn set_audience(&self, audience: &AudienceSet) -> Result<(), TransportError> {
        let local = self.shared.room.local_participant();
        let result = match audience {
            AudienceSet::All => local.set_track_subscription_permissions(true, Vec::new()).await,
            AudienceSet::Only(ids) => {
                let permissions = ids
                    .iter()
                    .map(|id| ParticipantTrackPermission {
                        participant_identity: ParticipantIdentity::from(id.name.clone()),
                        allow_all: true,
                        allowed_track_sids: Vec::new(),
                    })
                    .collect();
                local.set_track_subscription_permissions(false, permissions).await
            }
        };
        result.map_err(|e| TransportError::Other(e.to_string()))
    }

    async fn close(&self) {
        if let Err(e) = self.shared.room.close().await {
            warn!("closing the room: {e}");
        }
    }
}

/// How much queued audio is still waiting to be played out: seconds handed to libwebrtc minus seconds elapsed since
/// (the same accounting as `AudioSource.wait_for_playout` in LiveKit's Python SDK).
#[derive(Debug, Default)]
struct PlayoutClock {
    queued: Duration,
    last: Option<Instant>,
}

impl PlayoutClock {
    fn captured(&mut self, audio: Duration) {
        let now = Instant::now();
        let elapsed = self.last.map_or(Duration::ZERO, |last| now - last);
        self.queued = self.queued.saturating_sub(elapsed) + audio;
        self.last = Some(now);
    }

    fn remaining(&self) -> Duration {
        let elapsed = self.last.map_or(Duration::ZERO, |last| last.elapsed());
        self.queued.saturating_sub(elapsed)
    }
}

struct LiveKitVoice {
    source: NativeAudioSource,
    clock: PlayoutClock,
}

impl std::fmt::Debug for LiveKitVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveKitVoice").finish_non_exhaustive()
    }
}

#[async_trait]
impl AudioOut for LiveKitVoice {
    async fn play(&mut self, pcm48: &[i16]) -> Result<(), TransportError> {
        let frame_len = Duration::from_millis(10);
        for chunk in pcm48.chunks(OUT_FRAME) {
            let mut data = chunk.to_vec();
            data.resize(OUT_FRAME, 0);
            let frame = AudioFrame {
                data: data.into(),
                sample_rate: PLAY_RATE,
                num_channels: 1,
                samples_per_channel: OUT_FRAME as u32,
            };
            self.source
                .capture_frame(&frame)
                .await
                .map_err(|e| TransportError::Other(e.message))?;
            self.clock.captured(frame_len);
        }
        let rest = self.clock.remaining();
        if !rest.is_zero() {
            tokio::time::sleep(rest).await;
        }
        Ok(())
    }
}

/// A lock that keeps working after a panic elsewhere (the map stays consistent: every change is one insert or remove).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fluxer_participant_names() {
        let id = identity_named("user_1234567890123456789_abc_def");
        assert_eq!(id.user(), Some(pb_domain::UserId(1_234_567_890_123_456_789)));
        assert_eq!(id.person.map(|(_, c)| c.0).as_deref(), Some("abc_def"));
        for other in ["bot", "user_", "user_12", "user_12_", "user_x_1"] {
            assert_eq!(identity_named(other).person, None, "{other}");
        }
    }
}
