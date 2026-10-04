//! An in-process voice transport: rooms are keyed by community and channel, people speak sample-exact 16 kHz audio
//! straight into the bot's subscriptions (no codec in between), and what the bot plays is kept. For engine tests whose
//! expectations come from the exact samples (parity with the old bot) or that run without a server (the engine's
//! scenarios); the LiveKit tests cover the real transport.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use pb_domain::{ChannelId, ConnectionId, GuildId, UserId};
use pb_fluxer_api::VoiceGrant;
use pb_voice_api::{
    AudienceSet, AudioIn, AudioOut, ConnectOpts, Identity, LISTEN_RATE, PLAY_RATE, Participant, PcmChunk,
    RemoteAudioTrack, RoomEvent, TrackKey, TrackSource, TransportError, VoiceRoom, VoiceTransport,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

type RoomId = (GuildId, ChannelId);

/// One thing the bot played.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayedItem {
    /// 48 kHz.
    pub samples: Vec<i16>,
    /// Who could hear it (`None`: the bot had not said yet).
    pub audience: Option<AudienceSet>,
    pub started: Instant,
}

/// The bot's side of a room while it is connected.
#[derive(Debug)]
struct Bot {
    /// Tells this connection from a later one (a late close of an old room must not end the new one).
    connection: u64,
    events: mpsc::UnboundedSender<RoomEvent>,
}

#[derive(Debug, Default)]
struct Room {
    people: BTreeMap<Identity, RemoteAudioTrack>,
    bot: Option<Bot>,
    subscribed: HashMap<TrackKey, mpsc::UnboundedSender<PcmChunk>>,
    /// The bot may not publish its voice here.
    speak_denied: bool,
    audience: Option<AudienceSet>,
    played: Vec<PlayedItem>,
}

impl Room {
    fn tell_bot(&self, event: RoomEvent) {
        if let Some(bot) = &self.bot {
            let _ = bot.events.send(event);
        }
    }
}

/// The transport; clones share the rooms.
#[derive(Debug, Clone, Default)]
pub struct MemVoice {
    rooms: Arc<Mutex<HashMap<RoomId, Room>>>,
    connections: Arc<Mutex<u64>>,
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
        room.tell_bot(RoomEvent::ParticipantJoined(Participant {
            identity: key.participant.clone(),
            audio: vec![track],
        }));
        Mic {
            voice: self.clone(),
            room: (guild, channel),
            key,
        }
    }

    /// From now on the bot cannot publish its voice in this channel (as without the Speak permission).
    pub fn deny_speak(&self, guild: GuildId, channel: ChannelId) {
        self.rooms().entry((guild, channel)).or_default().speak_denied = true;
    }

    /// Whether the bot is connected to the room.
    pub fn has_bot(&self, guild: GuildId, channel: ChannelId) -> bool {
        self.rooms().get(&(guild, channel)).is_some_and(|r| r.bot.is_some())
    }

    /// Who hears the bot now, as it last set it (`None`: it never did, or it left).
    pub fn audience(&self, guild: GuildId, channel: ChannelId) -> Option<AudienceSet> {
        self.rooms().get(&(guild, channel)).and_then(|r| r.audience.clone())
    }

    /// The room drops the bot (a server restart, a network failure): its event stream ends with `Disconnected`.
    pub fn disconnect(&self, guild: GuildId, channel: ChannelId, reason: &str) {
        if let Some(room) = self.rooms().get_mut(&(guild, channel)) {
            room.tell_bot(RoomEvent::Disconnected { reason: reason.into() });
            room.bot = None;
            room.subscribed.clear();
            room.audience = None;
        }
    }

    /// Each thing the bot played in a channel, in order.
    pub fn played_items(&self, guild: GuildId, channel: ChannelId) -> Vec<PlayedItem> {
        self.rooms()
            .get(&(guild, channel))
            .map(|r| r.played.clone())
            .unwrap_or_default()
    }

    /// Everything the bot played in a channel, one item after another (48 kHz).
    pub fn played(&self, guild: GuildId, channel: ChannelId) -> Vec<i16> {
        self.played_items(guild, channel)
            .into_iter()
            .flat_map(|p| p.samples)
            .collect()
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

    /// Speaks 16 kHz audio in real time, in 20 ms pieces as a transport delivers them. Nothing arrives while muted.
    pub async fn say(&self, pcm: &[i16]) {
        let piece = (LISTEN_RATE / 50) as usize;
        let start = Instant::now();
        for (i, samples) in pcm.chunks(piece).enumerate() {
            let tx = self.voice.rooms().get(&self.room).and_then(|r| {
                let on = r.people.get(&self.key.participant).is_some_and(|t| !t.muted);
                r.subscribed.get(&self.key).filter(|_| on).cloned()
            });
            if let Some(tx) = tx {
                let _ = tx.send(PcmChunk {
                    samples: samples.to_vec(),
                    received: Instant::now().into_std(),
                });
            }
            let due = start + Duration::from_millis(20 * (i as u64 + 1));
            tokio::time::sleep_until(due).await;
        }
    }

    /// Mutes or unmutes the microphone; the bot is told, as by a transport.
    pub fn mute(&self, muted: bool) {
        let mut rooms = self.voice.rooms();
        let Some(room) = rooms.get_mut(&self.room) else {
            return;
        };
        if let Some(track) = room.people.get_mut(&self.key.participant) {
            track.muted = muted;
        }
        room.tell_bot(RoomEvent::TrackMuted {
            key: self.key.clone(),
            muted,
        });
    }

    /// Leaves the room: the bot's subscription ends and it is told.
    pub fn leave(self) {
        let mut rooms = self.voice.rooms();
        let Some(room) = rooms.get_mut(&self.room) else {
            return;
        };
        room.people.remove(&self.key.participant);
        room.subscribed.remove(&self.key);
        room.tell_bot(RoomEvent::ParticipantLeft(self.key.participant.clone()));
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
        let connection = {
            let mut n = self.connections.lock().unwrap_or_else(PoisonError::into_inner);
            *n += 1;
            *n
        };
        let (tx, rx) = mpsc::unbounded_channel();
        self.rooms().entry((*guild, *channel)).or_default().bot = Some(Bot { connection, events: tx });
        Ok((
            Box::new(MemRoom {
                voice: self.clone(),
                room: (*guild, *channel),
                connection,
            }),
            rx,
        ))
    }
}

struct MemRoom {
    voice: MemVoice,
    room: RoomId,
    connection: u64,
}

impl MemRoom {
    /// Runs `f` on the room while this is the bot's current connection to it.
    fn with_room<T>(&self, f: impl FnOnce(&mut Room) -> T) -> Option<T> {
        self.voice
            .rooms()
            .get_mut(&self.room)
            .filter(|r| r.bot.as_ref().is_some_and(|b| b.connection == self.connection))
            .map(f)
    }
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
        self.with_room(|room| {
            if !room.people.values().any(|t| &t.key == track) {
                return Err(TransportError::NoSuchTrack(track.clone()));
            }
            let (tx, rx) = mpsc::unbounded_channel();
            room.subscribed.insert(track.clone(), tx);
            Ok(rx)
        })
        .unwrap_or(Err(TransportError::Closed))
    }

    async fn unsubscribe(&self, track: &TrackKey) -> Result<(), TransportError> {
        self.with_room(|room| room.subscribed.remove(track));
        Ok(())
    }

    async fn publish_voice(&self) -> Result<Box<dyn AudioOut>, TransportError> {
        match self.with_room(|room| room.speak_denied) {
            None => Err(TransportError::Closed),
            Some(true) => Err(TransportError::NoSpeakPermission),
            Some(false) => Ok(Box::new(MemOut {
                voice: self.voice.clone(),
                room: self.room,
            })),
        }
    }

    async fn set_audience(&self, audience: &AudienceSet) -> Result<(), TransportError> {
        self.with_room(|room| room.audience = Some(audience.clone()))
            .ok_or(TransportError::Closed)
    }

    async fn close(&self) {
        self.with_room(|room| {
            room.bot = None;
            room.subscribed.clear();
            room.audience = None;
        });
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
            let item = PlayedItem {
                samples: pcm48.to_vec(),
                audience: room.audience.clone(),
                started: Instant::now(),
            };
            room.played.push(item);
        }
        tokio::time::sleep(Duration::from_secs_f64(pcm48.len() as f64 / f64::from(PLAY_RATE))).await;
        Ok(())
    }
}
