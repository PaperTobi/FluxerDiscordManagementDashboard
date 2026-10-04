//! Two bots one after another in the same process and runtime (as when the bot leaves and later rejoins calls).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

mod common;

use common::*;
use pb_store_api::{DecisionRecord, Event};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs LIVEKIT_SERVER and PB_WEIGHTS; run with --ignored"]
async fn one_after_another_in_one_runtime() {
    for round in 0..2 {
        let rig = Rig::start(Setup::default()).await;
        let (alice, mic) = rig.alice_joins().await;
        mic.say(&speech48("profane_1.wav")).await.unwrap();
        mic.say(&vec![0i16; 48_000 * 2]).await.unwrap();
        rig.wait_event(
            "a warned sentence",
            30,
            |e| matches!(e, Event::Sentence(s) if matches!(s.decision, DecisionRecord::Warn { .. })),
        )
        .await;
        let heard = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while rig.alice_heard(&alice) <= 1600 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await;
        if heard.is_err() {
            let bot = rig.fake.config().bot_id;
            let conns = rig.fake.bot_connections();
            let total = conns
                .first()
                .map_or(0, |(c, _, _, _)| alice.heard_from(&format!("user_{bot}_{c}")).len());
            let played: Vec<_> = rig
                .events()
                .await
                .into_iter()
                .filter(|e| matches!(e, Event::Played(_)))
                .collect();
            panic!(
                "round {round}: Alice did not hear the warning; samples received {total}, connections {conns:?}, played {played:?}"
            );
        }
        rig.stop(Some(alice)).await;
    }
}
