//! A throw-away local LiveKit server on free loopback ports, tokens shaped like the ones Fluxer issues, and scripted
//! participants that publish WAV audio or record what they hear.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use livekit::options::TrackPublishOptions;
use livekit::prelude::{LocalAudioTrack, LocalTrack, RemoteTrack, Room, RoomEvent, RoomOptions, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions};
use livekit::webrtc::prelude::AudioFrame;
use sha2::Sha256;

pub const API_KEY: &str = "devkey";
pub const API_SECRET: &str = "devsecret-devsecret-devsecret-0123456789";

/// The livekit-server binary: `LIVEKIT_SERVER`, else `livekit-server` on the `PATH`.
pub fn server_binary() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("LIVEKIT_SERVER") {
        return p.into();
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join("livekit-server"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| "livekit-server".into())
}

/// Whether integration tests that need a LiveKit server can run here.
pub fn available() -> bool {
    server_binary().is_file()
}

/// A port that is free for TCP and UDP right now.
pub fn free_port() -> Result<u16> {
    for _ in 0..50 {
        let tcp = TcpListener::bind("127.0.0.1:0")?;
        let port = tcp.local_addr()?.port();
        if UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    bail!("no free port")
}

/// A running livekit-server, killed on drop.
#[derive(Debug)]
pub struct LiveKitServer {
    pub port: u16,
    child: Child,
}

impl LiveKitServer {
    pub fn start() -> Result<Self> {
        let mut last = None;
        for _ in 0..4 {
            match Self::start_once() {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("livekit-server did not start")))
    }

    fn start_once() -> Result<Self> {
        let (port, tcp, udp) = (free_port()?, free_port()?, free_port()?);
        let config = format!(
            "port: {port}\nrtc:\n  tcp_port: {tcp}\n  udp_port: {udp}\n  use_external_ip: false\n  node_ip: 127.0.0.1\nkeys:\n  {API_KEY}: {API_SECRET}\n"
        );
        let mut child = Command::new(server_binary())
            .args([
                "--bind",
                "127.0.0.1",
                "--node-ip",
                "127.0.0.1",
                "--config-body",
                &config,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting livekit-server")?;
        for _ in 0..100 {
            if TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_ok() {
                return Ok(LiveKitServer { port, child });
            }
            if child.try_wait()?.is_some() {
                bail!("livekit-server exited at start-up");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        bail!("livekit-server did not start listening")
    }

    pub fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }
}

impl Drop for LiveKitServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A LiveKit access token with the grant shape Fluxer issues: join, subscribe, and publish only the microphone when
/// the account may speak.
pub fn mint(identity: &str, room: &str, can_speak: bool) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let header = serde_json::json!({"alg": "HS256", "typ": "JWT"});
    let claims = serde_json::json!({
        "iss": API_KEY,
        "sub": identity,
        "nbf": now.saturating_sub(10),
        "exp": now + 600,
        "video": {
            "roomJoin": true,
            "room": room,
            "canSubscribe": true,
            "canPublish": can_speak,
            "canPublishSources": if can_speak { vec!["microphone"] } else { Vec::new() },
        },
    });
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(API_SECRET.as_bytes()).expect("HMAC takes any key length");
    mac.update(signing_input.as_bytes());
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

/// A scripted participant (a stand-in for a person in the call). Subscribes only to the participant it listens to
/// (automatic subscription off, like the bot), records what it hears at 16 kHz mono, and counts how often someone
/// subscribed to its own microphone.
pub struct Person {
    pub room: Room,
    heard: Arc<Mutex<HashMap<String, Vec<i16>>>>,
    subscribed_to_me: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Person {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Person").finish_non_exhaustive()
    }
}

/// A published microphone; [`Mic::say`] streams audio into it in real time.
#[derive(Clone)]
pub struct Mic {
    source: NativeAudioSource,
}

impl std::fmt::Debug for Mic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mic").finish_non_exhaustive()
    }
}

impl Mic {
    /// Streams mono 48 kHz audio in 10 ms frames; returns when it has all been handed over.
    pub async fn say(&self, pcm48: &[i16]) -> Result<()> {
        for chunk in pcm48.chunks(480) {
            let mut data = chunk.to_vec();
            data.resize(480, 0);
            let frame = AudioFrame {
                data: data.into(),
                sample_rate: 48_000,
                num_channels: 1,
                samples_per_channel: 480,
            };
            self.source
                .capture_frame(&frame)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
        }
        Ok(())
    }
}

impl Person {
    /// Joins `room` as `identity`, listening to participants whose identity starts with `listens_to` (a whole
    /// identity, or a prefix like `user_<bot id>_` when the connection id is not known yet).
    pub async fn join(server: &LiveKitServer, identity: &str, room: &str, listens_to: &str) -> Result<Self> {
        let token = mint(identity, room, true);
        let mut options = RoomOptions::default();
        options.auto_subscribe = false;
        pb_voice_livekit::install_network().map_err(anyhow::Error::msg)?;
        let (room, mut events) = Room::connect(&server.url(), &token, options).await?;
        let listens_to = listens_to.to_owned();
        for participant in room.remote_participants().values() {
            if participant.identity().to_string().starts_with(&listens_to) {
                for publication in participant.track_publications().values() {
                    publication.set_subscribed(true);
                }
            }
        }
        let heard: Arc<Mutex<HashMap<String, Vec<i16>>>> = Arc::default();
        let subscribed_to_me = Arc::new(AtomicUsize::new(0));
        let (sink, counter) = (Arc::clone(&heard), Arc::clone(&subscribed_to_me));
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    RoomEvent::TrackPublished {
                        publication,
                        participant,
                    } if participant.identity().to_string().starts_with(&listens_to) => {
                        publication.set_subscribed(true);
                    }
                    RoomEvent::LocalTrackSubscribed { .. } => {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    RoomEvent::TrackSubscribed {
                        track: RemoteTrack::Audio(audio),
                        participant,
                        ..
                    } => {
                        let who = participant.identity().to_string();
                        let sink = Arc::clone(&sink);
                        let options = NativeAudioStreamOptions {
                            queue_size_frames: Some(0),
                        };
                        let mut stream = NativeAudioStream::with_options(audio.rtc_track(), 16_000, 1, options);
                        tokio::spawn(async move {
                            while let Some(frame) = stream.next().await {
                                sink.lock()
                                    .expect("not poisoned")
                                    .entry(who.clone())
                                    .or_default()
                                    .extend_from_slice(&frame.data);
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
        Ok(Person {
            room,
            heard,
            subscribed_to_me,
        })
    }

    /// Publishes a microphone track.
    pub async fn publish_mic(&self) -> Result<Mic> {
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
        let track = LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        let options = TrackPublishOptions {
            source: TrackSource::Microphone,
            ..Default::default()
        };
        self.room
            .local_participant()
            .publish_track(LocalTrack::Audio(track), options)
            .await?;
        Ok(Mic { source })
    }

    /// Everything heard from `identity` so far (16 kHz mono).
    pub fn heard_from(&self, identity: &str) -> Vec<i16> {
        self.heard
            .lock()
            .expect("not poisoned")
            .get(identity)
            .cloned()
            .unwrap_or_default()
    }

    /// Leaves the room.
    pub async fn leave(self) -> Result<()> {
        self.room.close().await?;
        Ok(())
    }

    /// How many times another participant subscribed to this person's microphone.
    pub fn times_subscribed_to(&self) -> usize {
        self.subscribed_to_me.load(Ordering::SeqCst)
    }
}
