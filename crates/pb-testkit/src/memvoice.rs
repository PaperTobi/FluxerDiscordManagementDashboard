//! An in-process voice transport: rooms are keyed by community and channel, people speak sample-exact 16 kHz audio
//! straight into the bot's subscriptions (no codec in between), and what the bot plays is kept. For engine tests whose
//! expectations come from the exact samples (parity with the old bot); the LiveKit tests cover the real transport.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use pb_domain::{ChannelId, ConnectionId, GuildId, UserId};
use pb_fluxer_api::VoiceGrant;
use pb_voice_api::{
    AudienceSet, AudioIn, AudioOut, ConnectOpts, Identity, LISTEN_RATE, PLAY_RATE, Participant, PcmChunk,
    RemoteAudioTrack, RoomEvent, TrackKey, TrackSource, TransportError, VoiceRoom, VoiceTransport,
};
use tokio::sync::mpsc;

type RoomId = (GuildId, ChannelId);

#[derive(Debug, Default)]
struct Room {
    people: BTreeMap<Identity, RemoteAudioTrack>,
    /// The bot's event stream while it is in the room.
    bot: Option<mpsc::UnboundedSender<RoomEvent>>,
    subscribed: HashMap<TrackKey, mpsc::UnboundedSender<PcmChunk>>,
    played: Vec<i16>,
}

/// The transport; clones share the rooms.
#[derive(Debug, Clone, Default)]
pub struct MemVoice {
    rooms: Arc<Mutex<HashMap<RoomId, Room>>>,
}

impl MemVoice {
    fn rooms(&self) -> MutexGuard<'_, HashMap<RoomId, Room>> {
        self.rooms.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Someone joins a voice channel with their microphone on, over their voice connection `connection`.
    pub fn join(&self, guild: GuildId, channel: ChannelId, user: UserId, connection: &str) -> Mic {
        let name = format!("user_{user}_{connection}");
        let key = TrackKey {
            participant: Identity {
                name: name.clone(),
                person: Some((user, ConnectionId(connection.to_owned()))),
            },
            track_sid: format!("TR_{name}"),
        };
        let track = RemoteAudioTrack {
            key: key.clone(),
            source: TrackSource::Microphone,
            muted: false,
        };
        let mut rooms = self.rooms();
        let room = rooms.entry((guild, channel)).or_default();
        room.people.insert(key.participant.clone(), track.clone());
        if let Some(bot) = &room.bot {
            let _ = bot.send(RoomEvent::ParticipantJoined(Participant {
                identity: key.participant.clone(),
                audio: vec![track],
            }));
        }
        Mic {
            voice: self.clone(),
            room: (guild, channel),
            key,
        }
    }

    /// Everything the bot played in a channel (48 kHz).
    pub fn played(&self, guild: GuildId, channel: ChannelId) -> Vec<i16> {
        self.rooms()
            .get(&(guild, channel))
            .map(|r| r.played.clone())
            .unwrap_or_default()
    }
}

/// A person's microphone in a room.
#[derive(Debug)]
pub struct Mic {
    voice: MemVoice,
    room: RoomId,
    key: TrackKey,
}

impl Mic {
    /// Whether the bot receives this microphone.
    pub fn heard(&self) -> bool {
        self.voice
            .rooms()
            .get(&self.room)
            .is_some_and(|r| r.subscribed.contains_key(&self.key))
    }

    /// Speaks 16 kHz audio in real time, in 20 ms pieces as a transport delivers them.
    pub async fn say(&self, pcm: &[i16]) {
        let piece = (LISTEN_RATE / 50) as usize;
        let start = Instant::now();
        for (i, samples) in pcm.chunks(piece).enumerate() {
            let tx = self
                .voice
                .rooms()
                .get(&self.room)
                .and_then(|r| r.subscribed.get(&self.key).cloned());
            if let Some(tx) = tx {
                let _ = tx.send(PcmChunk {
                    samples: samples.to_vec(),
                    received: Instant::now(),
                });
            }
            let due = start + Duration::from_millis(20 * (i as u64 + 1));
            tokio::time::sleep_until(due.into()).await;
        }
    }
}

#[async_trait]
impl VoiceTransport for MemVoice {
    async fn connect(
        &self,
        grant: &VoiceGrant,
        _opts: ConnectOpts,
    ) -> Result<(Box<dyn VoiceRoom>, mpsc::UnboundedReceiver<RoomEvent>), TransportError> {
        let (guild, channel) = match grant {
            VoiceGrant::LiveKit { guild, channel, .. } => (guild, channel),
            _ => return Err(TransportError::UnsupportedGrant),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        self.rooms().entry((*guild, *channel)).or_default().bot = Some(tx);
        Ok((
            Box::new(MemRoom {
                voice: self.clone(),
                room: (*guild, *channel),
            }),
            rx,
        ))
    }
}

struct MemRoom {
    voice: MemVoice,
    room: RoomId,
}

#[async_trait]
impl VoiceRoom for MemRoom {
    fn participants(&self) -> Vec<Participant> {
        self.voice.rooms().get(&self.room).map_or_else(Vec::new, |r| {
            r.people
                .values()
                .map(|t| Participant {
                    identity: t.key.participant.clone(),
                    audio: vec![t.clone()],
                })
                .collect()
        })
    }

    async fn subscribe(&self, track: &TrackKey) -> Result<AudioIn, TransportError> {
        let mut rooms = self.voice.rooms();
        let room = rooms.get_mut(&self.room).ok_or(TransportError::Closed)?;
        if !room.people.values().any(|t| &t.key == track) {
            return Err(TransportError::NoSuchTrack(track.clone()));
        }
        let (tx, rx) = mpsc::unbounded_channel();
        room.subscribed.insert(track.clone(), tx);
        Ok(rx)
    }

    async fn unsubscribe(&self, track: &TrackKey) -> Result<(), TransportError> {
        if let Some(room) = self.voice.rooms().get_mut(&self.room) {
            room.subscribed.remove(track);
        }
        Ok(())
    }

    async fn publish_voice(&self) -> Result<Box<dyn AudioOut>, TransportError> {
        Ok(Box::new(MemOut {
            voice: self.voice.clone(),
            room: self.room,
        }))
    }

    async fn set_audience(&self, _audience: &AudienceSet) -> Result<(), TransportError> {
        Ok(())
    }

    async fn close(&self) {
        if let Some(room) = self.voice.rooms().get_mut(&self.room) {
            room.bot = None;
            room.subscribed.clear();
        }
    }
}

/// The bot's voice: kept, and played out in real time.
struct MemOut {
    voice: MemVoice,
    room: RoomId,
}

#[async_trait]
impl AudioOut for MemOut {
    async fn play(&mut self, pcm48: &[i16]) -> Result<(), TransportError> {
        if let Some(room) = self.voice.rooms().get_mut(&self.room) {
            room.played.extend_from_slice(pcm48);
        }
        tokio::time::sleep(Duration::from_secs_f64(pcm48.len() as f64 / f64::from(PLAY_RATE))).await;
        Ok(())
    }
}
