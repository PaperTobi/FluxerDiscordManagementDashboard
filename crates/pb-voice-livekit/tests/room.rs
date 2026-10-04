//! M1 spike: the LiveKit transport against a real local LiveKit server (`LIVEKIT_SERVER`, skipped when missing).
//! Proves: automatic subscription is off, only the chosen microphone is received (16 kHz, same audio), the bot's
//! voice is published and the audience restriction is enforced by the server.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::time::Duration;

use pb_domain::{ChannelId, ConnectionId, GuildId};
use pb_fluxer_api::VoiceGrant;
use pb_testkit::audio::{best_correlation, envelope, read_wav, resample};
use pb_testkit::lk::{self, LiveKitServer, Person};
use pb_voice_api::{AudienceSet, ConnectOpts, RoomEvent, TrackSource, VoiceRoom, VoiceTransport};
use pb_voice_livekit::LiveKitTransport;

const ROOM: &str = "voice-room";
const BOT: &str = "user_99_bot";
const A: &str = "user_1_c1";
const B: &str = "user_2_c2";
const C: &str = "user_3_c3";

fn grant(server: &LiveKitServer) -> VoiceGrant {
    VoiceGrant::LiveKit {
        guild: GuildId(1),
        channel: ChannelId(2),
        connection: ConnectionId("bot".into()),
        endpoint: server.url().parse().expect("valid url"),
        token: lk::mint(BOT, ROOM, true).into(),
        e2ee_key: None,
    }
}

fn tone(seconds: f32, hz: f32) -> Vec<i16> {
    let n = (48_000.0 * seconds) as usize;
    (0..n)
        .map(|i| ((i as f32 / 48_000.0 * hz * std::f32::consts::TAU).sin() * 12_000.0) as i16)
        .collect()
}

fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

async fn wait_for_mic(
    room: &dyn VoiceRoom,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    who: &str,
) -> pb_voice_api::TrackKey {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        for p in room.participants() {
            if p.identity.name == who
                && let Some(t) = p.audio.iter().find(|t| t.source == TrackSource::Microphone)
            {
                return t.key.clone();
            }
        }
        tokio::select! {
            _ = events.recv() => {}
            _ = tokio::time::sleep_until(deadline) => panic!("{who} never published a microphone"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a livekit-server binary (LIVEKIT_SERVER); run with --ignored"]
async fn receives_only_the_chosen_microphone_and_controls_its_audience() {
    assert!(
        lk::available(),
        "no livekit-server at {}",
        lk::server_binary().display()
    );
    let server = LiveKitServer::start().expect("livekit-server starts");

    let (room, mut events) = LiveKitTransport
        .connect(&grant(&server), ConnectOpts::default())
        .await
        .expect("bot joins");
    let a = Person::join(&server, A, ROOM, BOT).await.expect("A joins");
    let b = Person::join(&server, B, ROOM, BOT).await.expect("B joins");
    let c = Person::join(&server, C, ROOM, BOT).await.expect("C joins");
    let a_mic = a.publish_mic().await.expect("A publishes");
    let b_mic = b.publish_mic().await.expect("B publishes");

    // Only A is "tracked": subscribe to A's microphone and nothing else.
    let a_track = wait_for_mic(room.as_ref(), &mut events, A).await;
    let _ = wait_for_mic(room.as_ref(), &mut events, B).await;
    assert_eq!(a_track.participant.user().map(|u| u.0), Some(1));
    let mut audio_in = room.subscribe(&a_track).await.expect("subscribe to A");

    let (speech16, rate) = read_wav(&pb_testkit::fixture("jfk.wav")).expect("fixture");
    assert_eq!(rate, 16_000);
    let speech48 = resample(&speech16, 16_000, 48_000).expect("resample");
    let collector = tokio::spawn(async move {
        let mut got = Vec::new();
        let end = tokio::time::Instant::now() + Duration::from_secs(14);
        while let Ok(Some(chunk)) = tokio::time::timeout_at(end, audio_in.recv()).await {
            got.extend_from_slice(&chunk.samples);
        }
        got
    });
    let b_noise = tone(11.0, 440.0);
    let (ra, rb) = tokio::join!(a_mic.say(&speech48), b_mic.say(&b_noise));
    ra.expect("A speaks");
    rb.expect("B speaks");
    let received = collector.await.expect("collector");

    let env_src = envelope(&speech16, 160);
    let env_got = envelope(&received, 160);
    let (r, lag) = best_correlation(&env_src, &env_got, 200);
    eprintln!(
        "received {:.1} s from A; envelope correlation {r:.3} at lag {} ms",
        received.len() as f32 / 16_000.0,
        lag * 10
    );
    assert!(received.len() as f32 / 16_000.0 > 9.0, "most of A's speech arrives");
    assert!(r >= 0.9, "received audio is A's speech (correlation {r:.3})");
    assert_eq!(
        b.times_subscribed_to(),
        0,
        "nobody subscribed to the untracked microphone"
    );
    assert_eq!(a.times_subscribed_to(), 1, "the bot subscribed to A");

    // The bot speaks to A only.
    let mut voice = room.publish_voice().await.expect("bot may speak");
    room.set_audience(&AudienceSet::Only(vec![pb_voice_livekit::identity_named(A)]))
        .await
        .expect("audience");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let started = std::time::Instant::now();
    voice.play(&tone(2.0, 1000.0)).await.expect("plays");
    let took = started.elapsed();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let a_heard = a.heard_from(BOT);
    let c_heard = c.heard_from(BOT);
    eprintln!(
        "play() took {took:?}; A heard {:.0} rms, C heard {:.0} rms",
        rms(&a_heard),
        rms(&c_heard)
    );
    assert!(
        took >= Duration::from_millis(1900),
        "play() returns after the audio played out"
    );
    assert!(rms(&a_heard) > 1000.0, "A hears the bot");
    assert!(rms(&c_heard) < 50.0, "C does not hear a warning meant for A");

    // Everyone may hear the bot now.
    room.set_audience(&AudienceSet::All).await.expect("audience");
    tokio::time::sleep(Duration::from_secs(1)).await;
    voice.play(&tone(2.0, 1000.0)).await.expect("plays");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let c_heard = c.heard_from(BOT);
    eprintln!("after audience=All, C heard {:.0} rms", rms(&c_heard));
    assert!(rms(&c_heard) > 1000.0, "C hears the bot once everyone may");

    room.close().await;
    for person in [a, b, c] {
        person.leave().await.expect("leaves");
    }
}
