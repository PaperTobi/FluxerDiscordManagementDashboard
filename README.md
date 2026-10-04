# Profanity Watch

![crates: 32](https://img.shields.io/badge/crates-32-orange) ![lines of Rust: 49k](https://img.shields.io/badge/lines%20of%20Rust-49k-blue) ![packages in Cargo.lock: 1079](https://img.shields.io/badge/Cargo.lock-1079%20packages-lightgrey) ![unsafe: 1 crate](https://img.shields.io/badge/unsafe-1%20crate-yellow) ![TODO: 0](https://img.shields.io/badge/TODO-0-brightgreen) ![warning clips: 5](https://img.shields.io/badge/warning%20clips-5-red) ![port: 8790](https://img.shields.io/badge/port-8790-informational) ![exit code 78: read the log](https://img.shields.io/badge/exit%20code%2078-read%20the%20log-critical)

A voice moderation bot for [Fluxer](https://fluxer.app). It follows chosen people into voice calls in your community,
listens **only to their microphones**, scores what they say with the Roblox voice-safety model (locally, on your
machine) and, when a sentence is flagged, warns them in the call: with a clip you recorded or uploaded, or with
text-to-speech in their language. Repeat violations can escalate (a message to you, mute, disconnect, time out), and a
mod-log channel gets a post for every flagged sentence.

Everything is managed in a **web page**: a live wall of who is talking and what the bot decided, communities, people,
settings, voice lines, reports and the audit trail. Chat commands (`!pb add @someone`, `!pb pause` …) cover the
everyday things.

It runs as one program (best in one **Podman** container, as an ordinary user), CPU only; a GPU is optional. The models
and voices are downloaded once, pinned to upstream revisions and checked against their SHA-256; nothing is downloaded
while the bot runs. Everything it keeps is in one data directory.

The bot is written in Rust. It replaces an earlier Python bot, whose data can be imported (see *Moving from the Python
bot*).

- [Requirements](#requirements)
- [1. Create the bot in Fluxer](#1-create-the-bot-in-fluxer)
- [2. Run it](#2-run-it): [Podman](#with-podman), [systemd / Fedora CoreOS](#with-systemd-quadlets-fedora-coreos),
  [without a container](#without-a-container)
- [3. Set it up in the web page](#3-set-it-up-in-the-web-page)
- [Using it](#using-it) · [Settings](#settings) · [HTTPS](#https) · [Everyday commands](#everyday-commands)
- [Moving from the Python bot](#moving-from-the-python-bot) · [Troubleshooting](#troubleshooting) ·
  [Privacy](#privacy-and-data) · [Development](#development) · [Licences](#licences)

**Everything else (mostly unnecessary):**

- [Prologue](#prologue)
- [Quick Facts Nobody Asked For](#quick-facts-nobody-asked-for)
- [Repository Statistics Dashboard](#repository-statistics-dashboard)
- [The Layer Cake](#the-layer-cake)
- [The Crate Gallery](#the-crate-gallery)
- [Biggest Files In The Repository](#biggest-files-in-the-repository)
- [The Five Warning Clips](#the-five-warning-clips)
- [Every Setting, With Commentary](#every-setting-with-commentary)
- [The Eight Labels](#the-eight-labels)
- [The Threshold Table](#the-threshold-table)
- [The Ladder Of Consequences](#the-ladder-of-consequences)
- [Chat Commands, Reviewed](#chat-commands-reviewed)
- [The CLI, Reviewed](#the-cli-reviewed)
- [Exit Codes](#exit-codes)
- [Tech Stack Trivia](#tech-stack-trivia)
- [Dependency Roster](#dependency-roster)
- [Cargo.lock Trivia](#cargolock-trivia)
- [Git History Trivia](#git-history-trivia)
- [Docs Folder Trivia](#docs-folder-trivia)
- [Container Lore](#container-lore)
- [Frequently Asked Questions Nobody Asked](#frequently-asked-questions-nobody-asked)
- [Haikus About The Repository](#haikus-about-the-repository)
- [Testimonials](#testimonials)
- [Imaginary Log Output](#imaginary-log-output)
- [Choose Your Own Adventure](#choose-your-own-adventure)
- [Minutes Of The Meeting That Never Happened](#minutes-of-the-meeting-that-never-happened)
- [Alphabet Of The Repository](#alphabet-of-the-repository)
- [Cheat Sheet](#cheat-sheet)
- [Wellness Checklist](#wellness-checklist)
- [Countdown To Release](#countdown-to-release)
- [Appendix A: Every Rust File In This Repository](#appendix-a-every-rust-file-in-this-repository)
- [Appendix B: Every Package In Cargo.lock](#appendix-b-every-package-in-cargolock)
- [Appendix C: Every Translation Key](#appendix-c-every-translation-key)
- [Appendix D: The Settings In Alphabetical Order Of Their Keys](#appendix-d-the-settings-in-alphabetical-order-of-their-keys)
- [Appendix E: Numbers That Appear In The Docs](#appendix-e-numbers-that-appear-in-the-docs)

## Prologue

Welcome to the README of **Profanity Watch**, a voice moderation bot that listens to your friends swear and tells them off with a recorded voice.

The first thing to know: this README is long. The second thing to know: most of it is not documentation. The real, useful parts are the numbered setup steps, the tables with commands, and the warnings about tokens. Everything else is, to put it kindly, *colour*.

**How to read this README:**

1. Skim the intro. It is the actual project description.
2. Skip to **Requirements** if you want to run the bot.
3. Ignore the rest unless you have time. You do not.

**Legend:**

| Symbol | Meaning |
| ------ | ------- |
| 🧠 | A fact about this repository that nobody asked for |
| 🦀 | Rust-related nonsense |
| 🐙 | Fluxer-related nonsense |
| 🎙️ | Voice-related nonsense |
| 📢 | A warning clip is mentioned |
| 🚫 | Do not do this |

## Quick Facts Nobody Asked For

All numbers below are real. We counted. We should not have, but we did.

- This repository has **32 crates** in `crates/`. That is a lot of crates for a bot that tells people to stop swearing.
- Those crates contain about **49,581 lines** of Rust across **235 files**. For comparison, this README alone is a weird fraction of that.
- The biggest crate, `pb-engine`, has **8,388 lines**. The smallest, `pb-voice-api`, has **161**. It is small, but it knows what it is doing.
- The biggest single file is `crates/pb-fluxer-fake/src/lib.rs` with **1,270 lines**: a fake Fluxer so the real one is not bothered during tests.
- `Cargo.lock` lists **1,079 packages**. The bot has 32 of its own. The rest are friends we met on the way.
- There are **173 tests** in the source, and **23** of them are marked `#[ignore]` because they need models, a LiveKit server or a browser. They are the introverts of the test suite.
- The word `TODO` appears **0 times** in the Rust code. Clippy's `todo` lint is set to `warn`. The author is either disciplined or lying.
- `unwrap()` appears **467** times (mostly in tests, we hope). Clippy's `unwrap_used` lint is also set to `warn`.
- `clone()` appears **859** times. Rust developers call this "being pragmatic".
- `async fn` appears **483** times. Waiting is the main job of this bot.
- There are **435** `struct` mentions, **144** `enum` mentions and **21** `trait` mentions. That is 3.0 structs per enum. Enums are the minority. Enums feel left out.
- The code has **2,091** `///` doc-comment lines. The author really wanted you to understand.
- The workspace forbids `unsafe_code` (set to `deny`). The only `unsafe` lives in the crate that talks to C, `pb-espeak`. It is the designated smoking area.
- The bot's default threshold is **0.6**. In a sense, it is 60% judgemental by default.
- The default web port is **8790**. Prime factors: 2 × 3 × 5 × 293. The number is not prime. Neither is the bot.
- The `Containerfile` has **3 stages** (`build`, `weights`, and the final image) and runs the bot as user `10001`, not root.
- The bot's output is **5 warning clips** totalling **13.09 seconds**. That is less time than it takes to read this bullet point out loud twice.
- The repository so far has **25 commits** on its main line, by 3 different authors, if you count the AI ones.
- Every single one of those commits was made on a **Sunday**. The bot is a Sunday project. The weekend was the product.
- The Rust toolchain file asks for **nightly** (with the wasm32 target), while `rust-version` in `Cargo.toml` says **1.99** and the edition is **2024**, resolver **3**. The code stays within stable Rust, so nightly is a lifestyle choice, not a need.
- The bot needs about **3 GB of RAM** and downloads about **1.7 GB** of model weights. Per kilobyte of RAM, it is shy.
- All user-facing text lives in **8 Fluent files**, **1,670 lines** in total, in two languages. Both languages say the same thing, just with different punctuation.

## Repository Statistics Dashboard

| Metric | Value |
| ------ | ----- |
| Crates | 32 |
| Rust files in `crates/` | 235 |
| Lines of Rust in `crates/` | 49,581 |
| Average lines per crate | 1,549 |
| Average lines per file | 210 |
| Packages in `Cargo.lock` | 1,079 |
| Direct third-party workspace dependencies | 52 |
| Tests | 173 |
| Ignored tests | 23 |
| `pub fn` | 694 |
| `async fn` | 483 |
| `Arc<` mentions | 246 |
| `Mutex` mentions | 129 |
| `format!` calls | 406 |
| `println!` calls | 81 |
| `///` doc lines | 2,091 |
| Settings with an English label | 49 |
| Translation keys (English) | 601 |
| Fluent lines (de + en) | 1,670 |
| Warning clips | 5 |
| Total warning audio | 13.09 s |
| Size of warning audio on disk | 1228 KiB |
| Model weights to download | ~1.7 GB |
| RAM at peak | ~3 GB |
| Build time | 30–60 min |
| Build disk | ~25 GB |
| Commits (main line) | 25 |
| Words in the real README | 3,020 |
| Words in this README | a lot more |

## The Layer Cake

The crates are sorted into layers, and the rules for who may depend on whom are checked by `cargo xtask deps` (see `xtask/layers.toml`). Imagine a cake. Every layer is a different flavour of responsibility, and the cake may only be eaten from the top.

| Layer | Crates | Dumb summary |
| ----- | ------ | ------------ |
| **L0** | `L0` | The pure ones. No I/O. They have never seen a network packet and are happy. |
| **L1** | `L0`, `L1` | The interfaces. Contract-minded. |
| **L2** | `L0`, `L1`, `L2` | The doers. This is where the models, Fluxer, voice, and storage live. |
| **L3** | `pb-fluxer`, `pb-store` | The brain and the gossip hub. |
| **L4** | `L2` | The web page and the web server. The face of the cake. |
| **L5** | `L0`, `L1`, `L2`, `L3`, `L4`, `L5` | The binary, the test kit, the fake Fluxer, the dev stack, and `xtask`. The sprinkles. |

```
        ┌──────────────────────────────────────────────┐
   L5   │  pb   testkit   fluxer-fake   devstack  xtask │  <- sprinkles
        ├──────────────────────────────────────────────┤
   L4   │          pb-web        pb-web-server          │  <- frosting
        ├──────────────────────────────────────────────┤
   L3   │            pb-live        pb-engine           │  <- cream
        ├──────────────────────────────────────────────┤
   L2   │  audio  espeak  vad  classifier  tts  infer   │
        │  fluxer  voice-livekit  store  import  tls     │  <- sponge
        │  weights                                       │
        ├──────────────────────────────────────────────┤
   L1   │  models-api  fluxer-api  voice-api  store-api  │  <- jam
        ├──────────────────────────────────────────────┤
   L0   │  domain settings segment policy voicelines     │
        │  commands i18n live-proto                       │  <- the plate
        └──────────────────────────────────────────────┘
```

The third-party crates are also confined. For example, `livekit` may only be used by `pb-voice-livekit` and `pb-testkit`; `burn` only by `pb-classifier-roblox` (the VAD was kicked out of the Burn club); `rten` only by `pb-tts-piper`; `turso` only by `pb-store` and `pb-import`. This is the software equivalent of a seating chart at a wedding.

## The Crate Gallery

Every crate in `crates/`, with its real line count and a personal opinion that nobody requested.

| Crate | Layer | Lines | Files | What it is | Opinion |
| ----- | ----- | ----: | ----: | ---------- | ------- |
| `pb` | L5 | 2,530 | 10 | The pb binary | The binary. The one that actually runs. Everyone else is a cheering section. |
| `pb-audio` | L2 | 541 | 4 | Audio decode, WAV, resampling, loudness, limiting, fades | Decode, resample, loudness, limiting, fades. Sound goes in, slightly different sound comes out. |
| `pb-classifier-roblox` | L2 | 1,143 | 8 | Roblox voice-safety-classifier v3 in Burn | The judge. Runs the Roblox voice-safety classifier in Burn. Quietly disapproves of you. |
| `pb-commands` | L0 | 435 | 2 | Chat command parser and permission levels (pure) | Parses `!pb`. So that typing 'add' feels like having power. |
| `pb-devstack` | L5 | 268 | 2 | A fake Fluxer instance with a local LiveKit server and people talking in a call, for trying the bot and its web UI (never shipped) | A fake Fluxer with fake friends talking in a fake call. Never shipped. Best social life in the repo. |
| `pb-domain` | L0 | 774 | 7 | Domain types: ids, labels, languages, scores, verdicts, decisions, scopes (pure, no I/O) | Ids, labels, scores, verdicts. The nouns of the project. Has no I/O and no regrets. |
| `pb-engine` | L3 | 8,388 | 28 | The bot: Fluxer session lifecycle, voice following, listening, decisions, warnings, moderation, reports, chat commands | The big one. Follows people into voice and decides what to do about them. |
| `pb-espeak` | L2 | 294 | 3 | espeak-ng (C library, pinned to Piper's commit) phonemization for Piper voices | Four C functions in a trench coat. The `unsafe` lives here and is not allowed out. |
| `pb-fluxer` | L2 | 2,584 | 6 | Fluxer client: discovery, gateway, REST with rate limits, OAuth2 | Talks to Fluxer. Respects rate limits. Is polite to the gateway. |
| `pb-fluxer-api` | L1 | 774 | 10 | The Fluxer interface the engine uses: gateway events, commands, REST operations, permissions | The shape of Fluxer, so nobody else has to know Fluxer. |
| `pb-fluxer-fake` | L5 | 1,270 | 1 | A fake Fluxer (discovery, REST, gateway with resume and voice joins, OAuth2) for tests | One file. Pretends to be an entire chat platform. A one-man theatre. |
| `pb-i18n` | L0 | 739 | 3 | Fluent bundles (de, en) for bot text and the web UI (pure) | Two languages (de, en). 1,670 lines of Fluent. Both are correct. |
| `pb-import` | L2 | 1,511 | 6 | One-time import of the Python bot's data directory | Eats the old Python bot's data. Silently. With respect for the dead. |
| `pb-infer` | L2 | 1,264 | 6 | Model threads, priority job queues and the inference handle | Model threads and priority job queues. Sentences queue up like at a bakery. |
| `pb-live` | L3 | 955 | 5 | The live-update hub: topic cells, consistent snapshots, deltas, per-connection sessions | The live-update hub. Basically the gossip department. |
| `pb-live-proto` | L0 | 1,876 | 8 | Live update wire protocol, client reducer, conveyor stage function (pure, wasm-safe) | The grammar of the gossip. Compiles to wasm so the browser can gossip too. |
| `pb-models-api` | L1 | 304 | 6 | Interfaces for the VAD, classifier and text-to-speech models, with contract tests | Interfaces for VAD, classifier and TTS, with contract tests. A prenuptial agreement for models. |
| `pb-policy` | L0 | 1,695 | 7 | Decider (strikes, escalation), FollowMachine (voice join/leave), channel policy (pure) | Strikes, escalation, follow machine. Judge Dredd, but with a config file. |
| `pb-segment` | L0 | 912 | 9 | Audio framing, PCM ring, hysteresis sentence segmenter, echo guard, windowing (pure) | Cuts speech into sentences. A very small butcher with a hysteresis. |
| `pb-settings` | L0 | 2,997 | 7 | Typed settings schema, layered resolution with sources, TOML editing (pure) | A typed schema with layered resolution. A lot of code to say 'it depends'. |
| `pb-store` | L2 | 2,864 | 16 | Hash-chained event log, Turso index, blob store, TOML settings, sessions | The hash-chained diary. Trusts nobody, including itself. |
| `pb-store-api` | L1 | 1,815 | 9 | Interfaces for the event log, query index, blobs, settings and sessions, with contract tests | The interface to the diary. A diary needs a front cover. |
| `pb-testkit` | L5 | 1,116 | 6 | Test helpers: local LiveKit server, tokens, participants, an in-process voice transport, stand-in models, golden data | Fake participants, local LiveKit, golden data. The cast of the test suite. |
| `pb-tls` | L2 | 218 | 2 | The bot's TLS client setup: graviola crypto (pure Rust), the system's trusted roots plus Mozilla's, a CPU check | Pure-Rust TLS setup. Also checks if your CPU is fancy enough. |
| `pb-tts-piper` | L2 | 836 | 6 | Piper text-to-speech on rten with espeak-ng phonemes | Makes the bot talk. Piper voices on rten with espeak-ng phonemes. The bot did not ask for a voice. |
| `pb-vad-silero` | L2 | 790 | 2 | Silero VAD v6.2, a hand-written forward pass | Voice activity detection with a hand-written forward pass. Answers 'human or fridge?' ten times faster than it used to. |
| `pb-voice-api` | L1 | 161 | 2 | Interface to the voice transport (rooms, tracks, audio in/out), with contract tests | The smallest crate. Proud of it. Interface to the voice transport. |
| `pb-voice-livekit` | L2 | 765 | 3 | Voice transport on the official LiveKit Rust SDK | Voice transport on the official LiveKit Rust SDK. Where audio enters and leaves. |
| `pb-voicelines` | L0 | 1,090 | 8 | Voice lines: slots, resolution, templates, utterance plans, prediction (pure) | Slots, templates, utterance plans. The bot's script writers. |
| `pb-web` | L4 | 4,163 | 24 | Leptos web app: pages (server-rendered) and islands (live parts, editors) | Leptos web app. Rust in the browser, on purpose. |
| `pb-web-server` | L4 | 4,124 | 17 | axum server: host allowlist, sessions and login, setup, forms, uploads, media, the live socket | axum. Host allowlist, sessions, login, uploads. The front door and the bouncer. |
| `pb-weights` | L2 | 385 | 2 | Pinned model and voice downloads with sha256 verification | Downloads about 1.7 GB, verifies the SHA-256 like a customs officer, and resumes if interrupted. |

**Crate size ranking, in words:**

1. `pb-engine`: 8,388 lines
2. `pb-web`: 4,163 lines
3. `pb-web-server`: 4,124 lines
4. `pb-settings`: 2,997 lines
5. `pb-store`: 2,864 lines
6. `pb-fluxer`: 2,584 lines
7. `pb`: 2,530 lines
8. `pb-live-proto`: 1,876 lines
9. `pb-store-api`: 1,815 lines
10. `pb-policy`: 1,695 lines
11. `pb-import`: 1,511 lines
12. `pb-fluxer-fake`: 1,270 lines
13. `pb-infer`: 1,264 lines
14. `pb-classifier-roblox`: 1,143 lines
15. `pb-testkit`: 1,116 lines
16. `pb-voicelines`: 1,090 lines
17. `pb-live`: 955 lines
18. `pb-segment`: 912 lines
19. `pb-tts-piper`: 836 lines
20. `pb-vad-silero`: 790 lines
21. `pb-domain`: 774 lines
22. `pb-fluxer-api`: 774 lines
23. `pb-voice-livekit`: 765 lines
24. `pb-i18n`: 739 lines
25. `pb-audio`: 541 lines
26. `pb-commands`: 435 lines
27. `pb-weights`: 385 lines
28. `pb-models-api`: 304 lines
29. `pb-espeak`: 294 lines
30. `pb-devstack`: 268 lines
31. `pb-tls`: 218 lines
32. `pb-voice-api`: 161 lines

## Biggest Files In The Repository

These are the thickest `.rs` files. They have eaten well.

| # | File | Lines |
| - | ---- | ----: |
| 1 | `crates/pb-fluxer-fake/src/lib.rs` | 1,270 |
| 2 | `crates/pb-settings/src/v1/values.rs` | 1,037 |
| 3 | `crates/pb-web-server/tests/routes.rs` | 928 |
| 4 | `crates/pb-engine/src/v1/engine.rs` | 904 |
| 5 | `crates/pb-live-proto/src/v1/state.rs` | 881 |
| 6 | `crates/pb-settings/src/v1/schema.rs` | 806 |
| 7 | `crates/pb-fluxer/src/v1/gateway.rs` | 791 |
| 8 | `crates/pb-vad-silero/src/lib.rs` | 709 |
| 9 | `crates/pb-fluxer/src/v1/rest.rs` | 693 |
| 10 | `crates/pb-policy/src/v1/follow.rs` | 675 |
| 11 | `crates/pb-engine/tests/scenarios.rs` | 674 |
| 12 | `crates/pb-infer/src/v1/mod.rs` | 658 |
| 13 | `crates/pb-store-api/src/v1/events.rs` | 632 |
| 14 | `crates/pb-store-api/src/v1/contract.rs` | 629 |
| 15 | `crates/pb-store/src/v1/log.rs` | 597 |
| 16 | `crates/pb-engine/src/v1/commands.rs` | 579 |
| 17 | `crates/pb-engine/src/v1/room.rs` | 569 |
| 18 | `crates/pb-i18n/src/v1/mod.rs` | 564 |
| 19 | `crates/pb-settings/src/v1/tree.rs` | 538 |
| 20 | `crates/pb-policy/tests/follow.rs` | 532 |
| 21 | `crates/pb-import/src/v1/mod.rs` | 532 |
| 22 | `crates/pb-fluxer/tests/client.rs` | 522 |
| 23 | `crates/pb-segment/src/v1/segmenter.rs` | 478 |
| 24 | `crates/pb-web-server/src/auth.rs` | 470 |
| 25 | `crates/pb-voicelines/src/v1/resolve.rs` | 464 |

## The Five Warning Clips

The repo ships exactly 5 warning clips in `clips/` (see `clips/clips.json`), each with weight 1.0. If nothing else is configured, the bot picks one of them at random whenever it tells someone off.

| File | Text | Length | Sample rate | Bits | Size |
| ---- | ---- | -----: | ----------: | ---: | ---: |
| `easy_on_swearing.wav` | "Easy on the swearing, please." | 2.33 s | 48,000 Hz | 16 | 219 KiB |
| `hey_watch_language.wav` | "Hey! Watch your language." | 3.36 s | 48,000 Hz | 16 | 315 KiB |
| `keep_it_clean.wav` | "Hey, keep it clean." | 2.14 s | 48,000 Hz | 16 | 201 KiB |
| `language_cut_it_out.wav` | "Language! Cut it out." | 3.00 s | 48,000 Hz | 16 | 281 KiB |
| `watch_your_mouth.wav` | "Watch your mouth, buddy." | 2.26 s | 48,000 Hz | 16 | 212 KiB |

**Total:** 13.09 seconds of disappointment, 1228 KiB on disk.

**Critical reviews:**

- 📢 *"Hey! Watch your language."* — The classic. The longest at 3.36 s, because it has an exclamation mark and a lot to say. ★★★★☆
- 📢 *"Language! Cut it out."* — Sounds like a PE teacher who has had enough. ★★★★★
- 📢 *"Easy on the swearing, please."* — The polite one. The "please" does a lot of work. ★★★☆☆
- 📢 *"Watch your mouth, buddy."* — The "buddy" is passive-aggressive. We love it. ★★★★★
- 📢 *"Hey, keep it clean."* — The shortest at 2.14 s. The haiku of warnings. ★★★★☆

**Fun with audio maths:**

- Playing all five clips back to back would take 13.09 seconds. Reading just the real part of this README out loud would take about 20 minutes. Reading all of it takes much longer. Do not do it.
- At 48,000 Hz mono, one second of uncompressed 16-bit audio is 96,000 bytes. The clips are all 48,000 Hz, so a clip of 2.33 s has roughly 111,840 samples. Each of them knows exactly what it is doing.
- Probability that a given warning is the "buddy" one: 1 in 5 (equal weights). That is 20%. Fate is cruel.
- If you upload your own clip in the web page, it is normalised and checked by the classifier. Yes, the classifier checks the warning clip for swearing. Yes, a warning that swears would be embarrassing.

## Tech Stack Trivia

Quick, unrequested notes on the things this bot is built from:

- 🦀 **Rust.** The language of the bot, the web page (via WebAssembly) and the build tool (`xtask`). Rust has a crab as a mascot called Ferris. The crab does not talk in voice calls.
- 🎙️ **Silero VAD.** Voice activity detection, version 6.2, in `pb-vad-silero`. Its job is to tell speech from non-speech. It used to run on Burn, but `docs/dependencies.md` says Burn spent most of each 0.2 ms step dispatching tiny operations, and the hand-written loops need about 0.02 ms. A tenfold speed-up by writing it yourself: the oldest trick in the book.
- 🧠 **Roblox voice-safety classifier.** v3, in Burn (`pb-classifier-roblox`). Roblox has a lot of experience with kids yelling in voice chat.
- 🗣️ **Piper.** A neural text-to-speech system, here on `rten` with `espeak-ng` as the phonemizer (`pb-tts-piper`, `pb-espeak`).
- 📡 **LiveKit.** The WebRTC platform for the voice calls. We use the official Rust SDK in `pb-voice-livekit`. Its libwebrtc is built against Chromium's libc++, which is why the build takes so long and the build machine feels so tired.
- 🗄️ **Turso.** A SQLite-compatible database written in Rust, used for the index. The repo vendors it under `third_party/turso` and `third_party/turso_sdk_kit`.
- 🔥 **Burn.** A deep learning framework in Rust. Vendored as `third_party/burn-flex` (about 1.5 MB). Today it runs the classifier only.
- 🌿 **branches.** A small vendored crate (MIT) in `third_party/branches`, patched because nightly renamed `core::intrinsics::abort` and turso's dependency could not follow. This repo now contains a crate called `branches` *and* several git branches. The two are not related, but the coincidence has been noted.
- 🌐 **axum.** The web server. Listens on 8790.
- 🍃 **Leptos.** The web UI framework. Server-rendered pages with interactive "islands".
- 📖 **Fluent.** Mozilla's localisation system for all texts, in `de` and `en`.
- 🐳 **Podman.** The recommended way to run the bot, as an ordinary user, with `--read-only --cap-drop=ALL --security-opt no-new-privileges`. It is a very polite container.
- 🧾 **systemd quadlets.** `deploy/quadlet/` has three units (`profanity-watch-data.volume`, `profanity-watch.build`, `profanity-watch.container`).
- 🔐 **graviola.** A pure-Rust crypto provider, used for TLS.
- 🐍 **Python.** The previous bot. Gone but not forgotten. The importer keeps its data alive.

**Where does the code *not* look like Rust?** The `docs/exceptions.toml` lists the few non-Rust pieces: LiveKit's libwebrtc and espeak-ng. The rest of the bot is pure, glorious, borrow-checked Rust. The exceptions are a list of 103 lines. Every other line is proud.

## Dependency Roster

The `Cargo.toml` of the workspace lists these direct third-party dependencies, with their pinned minor versions. Each of them gets a one-line review.

| Crate | Version | Review |
| ----- | ------- | ------ |
| `anyhow` | 1.0.104 | Makes errors easy. Makes error handling feel like a warm bath. |
| `async-trait` | 0.1.92 | Lets traits be async. Hides the pain. |
| `axum` | 0.8.9 | The web server framework. Says 'hello' to your browser on port 8790. |
| `base64` | 0.23.1 | Turns bytes into letters. The alphabet is 64 characters long and they all showed up. |
| `fluent-bundle` | 0.16.0 | Mozilla's localisation system. Powers 'Hey! Watch your language' in two languages. |
| `fluent-syntax` | 0.12.0 | Parses the `.ftl` files. Reads 1,670 lines without complaining. |
| `bytes` | 1.12.1 | Cheaply cloneable bytes. The bot has 800+ `clone()` calls and feels fine about it. |
| `futures` | 0.3.34 | Futures. Things that will be done later. Like the README. |
| `ebur128` | 0.1.10 | Loudness measurement. So the warning is not louder than the swearing. |
| `getrandom` | 0.4.3 | Asks the OS for random numbers. The OS gives a number. The bot says thanks. |
| `hmac` | 0.13.0 | Signs things so nobody fakes the cookie. |
| `jiff` | 0.2.37 | Dates and times. The bot cares what time zone you are in for the daily report. |
| `leptos` | 0.8.21 | Rust in the browser. The web page is written in the same language as the thing it controls. |
| `leptos_axum` | 0.8.10 | Glue between Leptos and axum. Glue is important. |
| `leptos_router` | 0.8.16 | Decides which page you see. The bouncer of the sidebar. |
| `wasm-bindgen` | 0.2.129 | Lets Rust and JavaScript talk. They mostly argue. |
| `js-sys` | 0.3.106 | JavaScript, but from Rust. Do not ask. |
| `web-sys` | 0.3.106 | The browser's API, but from Rust. Do not ask twice. |
| `tower` | 0.5.3 | Middleware for services. Layers of onions. |
| `tower-http` | 0.7.1 | HTTP bits for the onion. |
| `rubato` | 5.0.1 | Resamples audio. Turns 48,000 Hz into whatever the models want. |
| `reqwest` | 0.13.5 | HTTP client. Fetches the weights. Politely. |
| `rustls` | 0.23.45 | TLS in Rust. The padlock. |
| `rustls-graviola` | 0.4.0 | Pure-Rust crypto provider. Checks your CPU like a bouncer. |
| `rustls-platform-verifier` | 0.7.1 | Uses the system's trusted roots. Trust, but ask the OS. |
| `tokio-rustls` | 0.26.6 | TLS for tokio. The padlock, async edition. |
| `webpki-root-certs` | 1.0.9 | Mozilla's list of trusted roots. A very long guest list. |
| `secrecy` | 0.10.3 | Wraps secrets so they do not show up in logs. Your bot token wears a ski mask. |
| `sha2` | 0.11.0 | Computes SHA-256 for the weights. Customs officer for 1.7 GB. |
| `tokio` | 1.53.2 | The async runtime. The heartbeat. Zero hearts, one runtime. |
| `tracing` | 0.1.44 | Logging with structure. 'web UI listening' is a span's dream. |
| `tracing-subscriber` | 0.3.23 | Decides what logging looks like. |
| `url` | 2.5.8 | Parses URLs. A solved problem that is never solved. |
| `cargo_metadata` | 0.23 | Used by `cargo xtask`. Reads Cargo like a diary. |
| `chromiumoxide` | 0.9.1 | Drives Chromium for browser tests. A puppeteer for a puppeteer. |
| `clap` | 4.6.7 | Parses `pb doctor`, `pb health`, `pb store`. Shouts at you with a help text. |
| `config` | 0.15.27 | Reads `PB__SECTION__KEY` env vars. Double underscores. Deliberate. |
| `rustix` | 1.1.5 | Safe Unix calls. No `unsafe` for us. |
| `tracing-appender` | 0.2.5 | Writes the daily log files in `<data>/logs`. |
| `serde` | 1 | Serialises everything. The ancient serde. |
| `serde_json` | 1 | JSON. For the live socket and elsewhere. |
| `thiserror` | 2.0.21 | Makes error types. Cries in `Display`. |
| `symphonia` | 0.6.1 | Decodes whatever audio you upload. Almost. |
| `opus-decoder` | 0.1.1 | Decodes the Opus voice from Fluxer calls. Opus: not a penguin. |
| `tokio-stream` | 0.1.17 | Streams for tokio. Flowing like a creek. |
| `tokio-util` | 0.7.19 | Utilities for tokio. Drawers full of useful things. |
| `tokio-tungstenite` | 0.30.0 | WebSockets. The gateway and the live socket. |
| `toml` | 1.1 | Reads `config.toml` and `settings/*.toml`. |
| `fastrand` | 2 | Random numbers for the engine. Maybe it picks your warning clip. The engine is not telling. |
| `toml_edit` | 0.25.15 | Edits TOML and keeps your comments. A rare act of kindness. |
| `turso` | 0.8.1 | A Rust SQLite-compatible database for the index. Vendored under `third_party/`. |
| `unic-langid` | 0.9.6 | Language identifiers. `de` or `en`. Pick one. |

## Requirements

> 🧠 **Repo fact:** the build needs about 8 GB of RAM and 25 GB of disk mostly because of LiveKit's libwebrtc, which is built against Chromium's libc++. Your 32 crates barely register.


| | |
|---|---|
| System | Linux. For the container: **Podman 4.4+** (rootless is fine; 5.2+ for the systemd units in `deploy/quadlet/`) |
| CPU | x86-64 with AVX2, AES and BMI2 (most CPUs since about 2014), or ARMv8 with the crypto extensions |
| Memory | about **3 GB** at peak while running; building needs about 8 GB and 25 GB of disk |
| Network | outgoing internet including **UDP** (voice); **TCP 8790** reachable in your network for the web page |
| Fluxer | a bot application, and someone with **Manage community** or **Administrator** to invite it |

Run **one** bot per bot token.

## 1. Create the bot in Fluxer

> 🐙 **Fluxer fact:** the bot token has the shape `<application id>.<secret>`. In the repo it is wrapped with `secrecy`, so it never shows up in logs. It wears a ski mask.


1. In Fluxer open **User Settings → Applications**, create an application and copy its **Bot token**
   (`<application id>.<secret>`) and its **Client secret**. Keep both private; if they leak, reset them there.
2. Tell the people you will track that the bot listens to them (see *Privacy and data*).

The bot is invited to your community after the setup (step 3), with a link from its web page.

## 2. Run it

> 🦀 **Run fact:** the first start prints a setup code. The code is not in the repo, not in the Containerfile, and not in your heart. It is in the log.


Get the code:

```bash
git clone https://github.com/PaperTobi/FluxerDiscordManagementDashboard.git profanity-watch
cd profanity-watch
```

### With Podman

```bash
podman build -t profanity-watch .
podman volume create profanity-watch-data

podman run -d --name profanity-watch --restart=unless-stopped \
  -p 8790:8790 \
  --read-only --cap-drop=ALL --security-opt no-new-privileges \
  -v profanity-watch-data:/data:U \
  profanity-watch

podman logs -f profanity-watch
```

The first build takes a while (count on 30–60 minutes): it compiles the bot and its web page and downloads about
1.7 GB of model weights and voices. Later builds reuse their caches. When the log shows
`web UI listening addr=0.0.0.0:8790` and, on the first start, the **setup code**, go on with step 3.

Updating: `git pull && podman build -t profanity-watch .`, then remove and start the container again (the volume keeps
everything).

### With systemd (quadlets, Fedora CoreOS)

`deploy/quadlet/` has three units (rootless, recommended, or rootful; Podman 5.2+). They build the image from the
checkout in `~/profanity-watch`, keep the data in a volume, restart the bot when it fails and check its health.

```bash
git clone https://github.com/PaperTobi/FluxerDiscordManagementDashboard.git ~/profanity-watch
mkdir -p ~/.config/containers/systemd
cp ~/profanity-watch/deploy/quadlet/* ~/.config/containers/systemd/
systemctl --user daemon-reload
systemctl --user start profanity-watch     # builds the image first, then starts the bot
loginctl enable-linger $USER               # start at boot without a login
journalctl --user -u profanity-watch -f
```

Rootful: the same files in `/etc/containers/systemd/`, and `systemctl` without `--user`. After `git pull`:
`systemctl --user restart profanity-watch-build profanity-watch`. Options go in a drop-in next to the unit, for example
`~/.config/containers/systemd/profanity-watch.container.d/options.conf`:

```ini
[Container]
Environment=PB__LOGGING__LEVEL=debug
```

A container that exits with **78** (a permanent problem: configuration, missing model files, a CPU without the needed
instructions) or **3** (another bot uses the same volume) is not restarted until that is fixed.

### Without a container

You need the build tools from *Development* below. Then, one line at a time (each needs the one before it to have
worked):

```bash
cargo xtask espeak-ng                                         # the pinned espeak-ng (Piper's phonemizer)
cargo xtask web                                               # the web page's browser bundle → target/site
cargo build --release -p pb                                   # the bot → target/release/pb
target/release/pb fetch-weights --dest target/weights         # models and voices, about 1.7 GB, resumable

export PB_DATA=$PWD/data                                      # everything the bot keeps
export PB__WEB__SITE=$PWD/target/site
export PB__INFERENCE__WEIGHTS=$PWD/target/weights
export PB__INFERENCE__ESPEAK_DATA=$PWD/target/espeak-ng/share
export PB__INFERENCE__CLIPS=$PWD/clips
target/release/pb doctor                                      # checks all of the above
target/release/pb run
```

The same paths can go into `$PB_DATA/config.toml` instead (see *Settings*).

### Bot token and client secret from the environment (optional)

Normally both are entered in the web page's setup and kept in the data directory (`secrets.toml`, mode 0600). To manage
them yourself, set `PB_BOT_TOKEN` and `PB_CLIENT_SECRET` (or `PB_BOT_TOKEN_FILE` / `PB_CLIENT_SECRET_FILE` naming a
file). Values from the environment win, are never written to disk, and the page shows them as set by the environment.
With Podman secrets:

```bash
printf '%s' 'APP_ID.SECRET' | podman secret create profanity-watch-token -
printf '%s' 'CLIENT_SECRET' | podman secret create profanity-watch-client-secret -
podman run … --secret profanity-watch-token,type=env,target=PB_BOT_TOKEN \
             --secret profanity-watch-client-secret,type=env,target=PB_CLIENT_SECRET …
```

(Quadlet: the `Secret=` lines shown in `profanity-watch.container`, in a drop-in.)

## 3. Set it up in the web page

> 🍃 **Web fact:** the web UI is written in Leptos, i.e. in Rust, compiled to WebAssembly for the browser. The shipped JavaScript is gated by `cargo xtask ci` ("the zero-C and shipped-JavaScript gates"). The author fears JavaScript.


Open **`http://<the machine's IP>:8790`** from your network. The setup asks, one step at a time:

1. **Setup code**: from the log, or `podman exec profanity-watch pb setup-code`.
2. **Fluxer instance**: keep `https://api.fluxer.app`, or your own instance's API address.
3. **Bot token** (skipped when it comes from the environment). The bot logs in at once and says whether Fluxer took it.
4. **Client secret**. The page shows the **redirect address** (for example `http://192.168.1.50:8790/auth/callback`):
   add it in Fluxer under your application's **Redirect URIs**. Give the machine a fixed IP so it keeps matching (if it
   does not, the login says which address to register).
5. **Log in with Fluxer**. Whoever logs in now becomes the bot's owner. That finishes the setup.

Then:

1. **Invite the bot**: *+ Invite the bot* in the sidebar opens Fluxer with exactly the permissions it needs (View
   Channel, Send Messages, Add Reactions, Attach Files, Read Message History, Connect, Speak; with moderation actions on
   also Mute Members, Move Members and Time Out Members). Someone with **Manage community** there approves it.
2. Look at the community's **Overview**: it lists what the bot may do in each voice channel and in the mod-log channel,
   and what is missing (a channel override can take permissions away).
3. Add people under **Track someone** (type a name, an ID, or paste a mention). To try it out silently first, switch
   on **Observe only (silent)**: the bot scores and records but says nothing.

Who may log in: the bot's owner (and *Extra bot owners* set under *System*), and anyone with **Manage community**,
**Administrator** or one of the *Admin roles* in a community the bot is in; they see only their communities. Owners
stay logged in for 12 hours, admins for 7 days; changing secrets needs a login from the last 15 minutes.

## Using it

> 🎙️ **Usage fact:** if all 5 warning clips are played back to back, the bot speaks for 13.09 seconds, which is less than a TikTok.


- **Live**: one tile per person the bot listens to (microphone level, where their latest sentence is: speaking, cut,
  queued, scored, decided; the time to a verdict), and the latest violations.
- **Communities**: calls and who is in them (muted or deafened people are marked: a deafened person hears no warning),
  the tracked people, violations; tabs for voice lines, settings and reports.
- **A person**: live view (sentences as they are scored, counts, what the bot said and did, *Say now*), history per
  day, recordings, their own voice lines and settings.
- **Voice lines**: the clip library (upload any audio or video file, or record in the browser when the page is opened
  over HTTPS or on localhost; each clip is normalised and checked by the classifier) and what the bot says, per line
  and language, globally, per community or per person. `{name}` in a text is the person's name. *Say* presets are lines
  for *Say now*.
- **Reports**: violations with every score against its bar, the swear jar, the owner's report and whether it arrived.
  **Audit**: every change, login, action and message, by kind and community. **System**: status, models, queues,
  storage, voices, Fluxer and secrets, global settings.

What the bot tells: the community's **mod log** (when set) gets one post per flagged sentence, with the escalation step
and the result of a moderation action; the **owner** gets a direct message for violations at escalation steps marked
*Tell the bot owner* (all of them by default), with the recording unless *Recordings in messages to the bot owner* is
off; the daily or weekly **report** is a direct message too.

The page updates live over one connection per tab; a tab in the background keeps only the sidebar current and catches
up at once when you come back.

### Chat commands

Type them in a text channel the bot can see, starting with `!pb` or a mention of the bot; `!pb help` lists everything.
Anyone: `!pb status`, `!pb list`, `!pb jar [@user]`. Admins: `!pb add @a`, `!pb remove @a`, `!pb pause` / `resume`,
`!pb observe on|off`, `!pb set threshold 0.6 [@a]`, `!pb set strikes 2`, `!pb set window 20s`,
`!pb set audience offender|tracked|channel`, `!pb set language de`, `!pb reset <setting|all> [@a]`,
`!pb modlog #channel|off`. German words work too (`an`/`aus`, `ja`/`nein`).

## Settings

> 🎛️ **Settings fact:** there are 49 settings with an English label. See *Every Setting, With Commentary* below for a review of each.


Settings are set globally, per community and per person; the most specific one wins (person > community > global >
`config.toml` > built-in), and the page shows where each value comes from. `pb settings docs` prints all of them.
Durations are written like `20s`, `5m`, `2h`, `1d`, or `unlimited` where that is allowed. There are no caps on counts or
lengths; the only limits are the model's and Fluxer's (a time-out lasts at most 365.25 days).

They are kept in `settings/*.toml` in the data directory (comments survive edits in the page). After editing the files
by hand, press *System → Read the settings files again* or send `SIGHUP`.

### `config.toml` and the environment

`<data>/config.toml`, or `PB__<SECTION>__<KEY>` environment variables, set how the process runs. Everything is
optional; the image sets the paths already.

```toml
[web]
bind = "0.0.0.0:8790"
# site = "/opt/pb/site"                                              # the web page's files
# tls = { cert = "/data/tls/cert.pem", key = "/data/tls/key.pem" }   # serve HTTPS (see below)

[inference]
device = "cpu"            # "gpu": the first discrete GPU through Vulkan (pass the GPU into the container)
# weights = "/opt/pb/weights"   espeak_data = "/opt/pb/espeak"   clips = "/opt/pb/clips"

[logging]
level = "info"            # RUST_LOG wins when set
# dir = "/data/logs"      # daily files, kept

[defaults]                # starting values for settings, below the global ones
threshold = 0.6
```

## HTTPS

> 🔐 **HTTPS fact:** the TLS layer of the bot's own outgoing connections uses the pure-Rust `graviola` crypto provider with the system's trusted roots plus Mozilla's. The page's HTTPS uses your PEM files.


Browsers record from the microphone only on secure pages, and logins are safer over HTTPS. Give the bot a certificate
and its key (PEM files, for example from your own CA, mkcert, `tailscale cert` or a DNS-validated Let's Encrypt
certificate) and it serves HTTPS on the same port. With Podman secrets:

```bash
podman secret create profanity-watch-tls-cert cert.pem
podman secret create profanity-watch-tls-key key.pem
podman run … \
  --secret profanity-watch-tls-cert,type=mount,target=/run/secrets/tls-cert,uid=10001 \
  --secret profanity-watch-tls-key,type=mount,target=/run/secrets/tls-key,uid=10001,mode=0400 \
  -e PB__WEB__TLS__CERT=/run/secrets/tls-cert -e PB__WEB__TLS__KEY=/run/secrets/tls-key \
  profanity-watch
```

(Quadlet: the same as `Secret=` and `Environment=` lines in a drop-in, see `profanity-watch.container`.) The
certificate must name the address you open the page by (host name or IP). Register `https://…/auth/callback` as the
redirect address in Fluxer and set *System → Web UI address* to match. Behind a reverse proxy that terminates TLS,
leave this off; the proxy must send `X-Forwarded-Proto: https`.

## Everyday commands

> 🧰 **Command fact:** `pb doctor` has the best name of all subcommands. It does not prescribe antibiotics.


With Podman (`podman exec profanity-watch pb …`), or `pb …` directly without a container:

| Task | Command |
|---|---|
| Log | `podman logs -f profanity-watch` (also daily files in `<data>/logs`) |
| Health | `pb health` |
| Check the installation | `pb doctor` |
| Setup code | `pb setup-code` |
| Lost access / redo the setup | `pb reset-setup`, then restart (token, secret, settings and data stay) |
| Check the event log | `pb store verify` |
| Rebuild the search index | stop the bot, then `podman run --rm -v profanity-watch-data:/data:U profanity-watch store rebuild-index` |
| Back up | `podman volume export profanity-watch-data -o pb-data.tar` |

## Moving from the Python bot

> 🐍 **Migration fact:** the old bot was Python. The new one is Rust. The importer is the only crate whose whole job is saying goodbye.


Import the old data directory into a new, empty volume before the first start:

```bash
podman run --rm -v profanity-watch-data:/data:U -v proofanitybot-data:/old:ro profanity-watch import --from /old
```

Settings, tracked people, voice-line texts and clips, history and violations, recordings, the audit trail, swear-jar
counts, pending timed mutes, installed voices and the secrets are carried over; `import-report.txt` lists what was done
and which old settings no longer exist (the old caps).

## Troubleshooting

> 🛠️ **Troubleshooting fact:** exit code 78 is a lot of the problems. 78 is `EX_CONFIG` in the old `sysexits.h`. The BSD people thought of this before you did.


| What you see | What to do |
|---|---|
| The page does not open | container running (`podman ps`)? same network? firewall (`8790/tcp`)? open it by IP |
| "This address is not one of the bot's web UI addresses" | open it by IP, then set *System → Web UI address* or *Extra host names* |
| Login fails at Fluxer | the redirect address is not registered exactly (setup step 4; the login names the address), or the client secret is wrong (enter it again on the setup's last step, or *System → Client secret*) |
| "Record a clip" is greyed out | the page is not opened over HTTPS (see *HTTPS*) or on localhost; upload a file instead |
| Exit code 78 | the log says why (configuration, model files, CPU); `pb doctor` checks everything |
| Exit code 3 | another bot process uses the same data directory |
| `rustc: symbol lookup error: …librustc_driver….so: undefined symbol …` | the distribution's Rust package does not match its LLVM libraries (a partial update, or packages from different repositories): install rustup instead (*Development*) |
| `rustup could not choose a version of cargo to run` | `rustup default nightly`, then in the project directory `rustup toolchain install` |
| `can't find crate for core` … `wasm32-unknown-unknown` | the browser target is missing: in the project directory `rustup toolchain install` (or `rustup target add wasm32-unknown-unknown`) |
| `target/release/pb`: unknown command / no such file | the build before it failed: scroll up to its first error |
| *System* says the instance has voice turned off | that Fluxer instance has no voice calls; nothing for the bot to do there |
| Does not join voice | person not tracked or paused, missing Connect, or an end-to-end encrypted call (setting *Join end-to-end encrypted calls*) |
| Flagged but no warning | strikes not reached yet, *Observe only (silent)* is on, the person is deafened, or the bot may not speak (it writes in the chat instead) |

## Privacy and data

> 🔍 **Privacy fact:** the event log is append-only and hash-chained. If you edit one entry by hand, `pb store verify` will find it. The hash chain is the repo's snitch.


Audio is processed in memory, on your machine. A sentence's recording is kept only when it was **flagged** (the owner
can switch *Recordings* to every sentence or to none); recordings stay until the owner deletes them (person page →
Recordings). Scores, decisions and every change are kept in an append-only, hash-chained event log in the data
directory; nothing is deleted automatically. Recordings go to the owner's direct messages and, if enabled, to the
mod-log channel. The bot is visible in the call while it listens.

**Recording or analysing people's voices may need their consent where you live: tell the people you track.**

## Development

> 💻 **Development fact:** `cargo xtask ci` runs fmt, clippy (native and wasm), tests, cargo-deny, cargo-shear, the layer rules, and the zero-C and shipped-JavaScript gates. That is a lot of gates. Nobody gets in.


Tools: Rust nightly through [rustup](https://rustup.rs) (`rust-toolchain.toml` names it, with the
`wasm32-unknown-unknown` target for the browser bundle; a stable Rust of at least `rust-version` in `Cargo.toml` works
too), clang 21+ and lld (LiveKit's libwebrtc is built against Chromium's libc++), glib
headers and pkg-config (libwebrtc), cmake and ninja (espeak-ng), git, and for the checks
`cargo install --locked cargo-deny cargo-shear`.

```bash
# Arch, CachyOS, Manjaro (rustup replaces the distribution's `rust` package)
sudo pacman -S --needed base-devel rustup clang lld pkgconf glib2 cmake ninja git
# Debian, Ubuntu: rustup from https://rustup.rs; clang 21 from https://apt.llvm.org (the Containerfile's build stage
# lists the packages)

rustup default nightly       # a Rust for everything else (without it: "rustup could not choose a version")
rustup toolchain install     # in this directory: nightly and the wasm32 target, as rust-toolchain.toml says
```

A distribution's own Rust package works only if it is recent enough, has the wasm32 target and matches the system's
LLVM libraries; rustup brings its own and avoids all three problems.

```bash
cargo xtask espeak-ng        # the pinned espeak-ng, into target/espeak-ng
cargo xtask web              # the browser bundle, into target/site (again after any change in crates/pb-web)
cargo build --release -p pb
cargo xtask ci               # fmt, clippy (native and wasm), tests, cargo-deny, cargo-shear, layer rules,
                             # the zero-C and shipped-JavaScript gates
cargo xtask freshness        # is every dependency on its newest release and maintained?
```

Tests that need the models, a LiveKit server or a browser are opt-in:

```bash
cargo run -p pb -- fetch-weights --dest target/weights      # once (PB_WEIGHTS points elsewhere)
LIVEKIT_SERVER=/path/to/livekit-server PB_CHROMIUM=chromium cargo test --workspace -- --ignored
```

`livekit-server` comes from [LiveKit's releases](https://github.com/livekit/livekit/releases) (or put it on the `PATH`).
`cargo run -p pb-devstack -- --data /tmp/pb-dev --ready` starts a fake Fluxer with a LiveKit server and two people
talking, for trying the bot and its page without a real community.

Every part is its own crate behind a versioned (`v1`) interface; `docs/design.md` describes the architecture,
`docs/fluxer-api.md` what the bot relies on from Fluxer, `docs/dependencies.md` the dependency choices and
`docs/exceptions.toml` the few non-Rust pieces (LiveKit's libwebrtc, espeak-ng).

## Every Setting, With Commentary

There are **49** settings with an English label. Real descriptions are in the web page and in `pb settings docs`. The commentary below is a service nobody ordered.

Reminder from the real docs: settings can be set globally, per community and per person; the most specific one wins (person > community > global > `config.toml` > built-in).

| Setting (English label) | Key | What it does, dumber |
| ----------------------- | --- | -------------------- |
| Warn about { $label } | `setting-label-enabled` | One switch per label. Which kinds of bad do we care about today? |
| Threshold for { $label } | `setting-label-threshold` | One bar per label. How bad is too bad. |
| Paused | `setting-paused` | The bot takes a nap. It still has feelings. It just does not act on them. |
| Only these communities | `setting-guild-allowlist` | A guest list for communities. No invite, no entry. |
| Tracked in every community | `setting-tracked-everywhere` | Track someone in every community. Surveillance, but systematic. |
| Join end-to-end encrypted calls | `setting-allow-e2ee-downgrade` | Join end-to-end encrypted calls anyway. Warning: security implications, read the tooltip. |
| Join delay | `setting-join-settle` | Wait a moment before joining voice. Politeness: the setting. |
| Leave delay | `setting-leave-grace` | Wait a moment before leaving. The bot hates awkward exits. |
| General threshold | `setting-threshold` | The general bar. Lower is stricter. Default 0.6. |
| Strikes before a warning | `setting-strikes` | How many offences before a warning. A baseball-adjacent number. |
| Strike window | `setting-strike-window` | How long strikes count. Strikes expire, like milk, but slower. |
| Pause that ends a sentence | `setting-end-silence` | How long a pause ends a sentence. The bot has strong opinions on commas. |
| Longest sentence | `setting-max-sentence` | The longest a sentence may be. Run-on sentences are cut off like in school. |
| Shortest speech scored | `setting-min-voiced` | The shortest speech worth scoring. 'Hm' is not a sentence. |
| Latest warning | `setting-max-reaction-delay` | How late a warning may still arrive. Too late and it is just rude. |
| Observe only (silent) | `setting-observe-only` | Silent mode. The bot judges you and tells nobody. |
| Who hears the warning | `setting-audience` | Who hears the warning: the offender, the tracked people, or the channel. Public shaming dial. |
| Warning volume | `setting-volume-db` | Warning volume in dB. Not 'yelling'. Just 'persuasive'. |
| Spoken language | `setting-voice-language` | Which language the warning speaks. `auto` guesses, and sometimes guesses German. |
| Fallback languages | `setting-fallback-languages` | Plan B for languages. And plan C. And plan D. |
| Text-to-speech voices | `setting-tts-voices` | Which text-to-speech voices are installed. A choir of robots. |
| Speech rate | `setting-speech-rate` | How fast the robot talks. Slow means polite. Fast means 'I have places to be'. |
| Without the Speak permission | `setting-no-speak-policy` | What to do without the Speak permission: write in chat, or log. The bot can only whisper. |
| Announce strikes | `setting-strike-notice` | Announce strikes. 'That is one.' |
| Announce actions | `setting-announce-actions` | Announce mutes and disconnects. Because the silence needs an explanation. |
| Count violations over | `setting-violation-window` | Count violations over a period. Memory has a length. |
| Escalation steps | `setting-escalation` | The ladder of consequences. See the ladder. |
| Allow moderation actions | `setting-actions-enabled` | Allow moderation actions at all. The big red button. |
| Greeting | `setting-greet-enabled` | Greeting. The bot says hello. It does this to be nice. It is not nice. |
| Mod log channel | `setting-modlog-channel` | Where the bot snitches. Every flagged sentence gets a post. |
| Audio in the mod log | `setting-modlog-audio` | Attach the audio in the mod log. Evidence, with sound. |
| Recordings in messages to the bot owner | `setting-owner-dm-audio` | Send recordings to the owner in direct messages. Think of it as postcards. |
| Summary report | `setting-digest` | Daily or weekly summary report. A newsletter nobody subscribed to. |
| Report time | `setting-digest-time` | What time the summary arrives. Not 3 a.m. Probably. |
| Report day (weekly) | `setting-digest-weekday` | Which day the weekly report arrives. Pick wisely. Not Friday afternoon. |
| Time zone | `setting-timezone` | Time zone. A source of all bugs in all projects. |
| Swear jar | `setting-jar-enabled` | The swear jar. A counter. No actual money. Calm down. |
| Chat language | `setting-chat-language` | The language the bot writes in. `de` or `en`. |
| Recordings | `setting-recordings` | Keep recordings for flagged, all, or no sentences. The privacy dial. |
| Community admins may play recordings | `setting-admins-play-audio` | Whether community admins may play recordings. Trust, but gated. |
| Chat commands | `setting-commands-enabled` | Chat commands on or off. Silence the `!pb` crowd. |
| Command prefix | `setting-command-prefix` | The prefix. Default `!pb`. Do not set it to a space. The validator will cry. |
| Extra bot owners | `setting-admin-user-ids` | Extra bot owners. More hands, more risk. |
| Admin roles | `setting-admin-role-ids` | Admin roles. Titles matter. |
| Fluxer instance | `setting-instance` | Which Fluxer instance. `https://api.fluxer.app` or your own. |
| Web UI address | `setting-ui-url` | The web UI address. Must match the redirect address. Matters. |
| Extra host names | `setting-allowed-hosts` | Extra host names the page may be opened by. A guest list for URLs. |
| CPU threads for the model | `setting-cpu-threads` | CPU threads for the model. More threads, more heat. |
| CPU threads for speech | `setting-tts-threads` | CPU threads for speech. The robot voice needs cores too. |

**The nine settings sections in the web page:** Tracking, Detection, Warning, Escalation, Greeting, Reporting, Recording, Chat commands and System. If you read them in that order, it tells a story: *we track you, we detect you, we warn you, we escalate, we greet, we report, we record, we talk about it in chat, and then we fix the system.*

## The Eight Labels

The classifier scores speech against eight labels. Each one can be turned on or off and has its own threshold (`setting-label-enabled`, `setting-label-threshold`).

| Label | Remark |
| ----- | ------ |
| Asking for personal info | Asking for personal info: 'what's your address' in a voice call. Please do not. |
| Discriminatory | Discriminatory. Not funny, not allowed. |
| Harassment | Harassment. The bot is not your friend today. |
| Sexual content | Sexual content. We are not making a joke here. The classifier has heard things. |
| Illegal and regulated | Illegal and regulated. The bot is not a lawyer, only a mildly judgemental listener. |
| Dating and romance | Dating and romance. Yes, the bot can flag you flirting. Yes, this is awkward. |
| Profanity | Profanity. The entire reason the repo exists. The star of the show. |
| Disruptive audio | Disruptive audio. Screaming, air horns, and your cousin's karaoke. |

## The Threshold Table

Lower is stricter. The default is 0.6. The table below is a feelings chart.

| Threshold | Strictness | How the bot feels |
| --------: | ---------- | ----------------- |
| 0.05 | 🔥🔥🔥🔥🔥 | Flags the sound of a sneeze. |
| 0.10 | 🔥🔥🔥🔥🔥 | Flags a sigh. A heavy one. |
| 0.15 | 🔥🔥🔥🔥🔥 | Flags 'oh no'. |
| 0.20 | 🔥🔥🔥🔥🔥 | Flags 'oh my gosh'. |
| 0.25 | 🔥🔥🔥🔥 | Flags 'darn'. |
| 0.30 | 🔥🔥🔥🔥 | Flags 'shoot'. |
| 0.35 | 🔥🔥🔥🔥 | Flags 'frick'. |
| 0.40 | 🔥🔥🔥🔥 | Flags the word 'frickin'. |
| 0.45 | 🔥🔥🔥 | Flags most mild language. |
| 0.50 | 🔥🔥🔥 | Strict, but fair. |
| 0.55 | 🔥🔥🔥 | Almost the default. |
| 0.60 | 🔥🔥🔥 | The default. The bot at peace. |
| 0.65 | 🔥🔥 | Lets some things slide. |
| 0.70 | 🔥🔥 | Lets many things slide. |
| 0.75 | 🔥🔥 | Only the clear cases. |
| 0.80 | 🔥 | Only the obvious ones. |
| 0.85 | 🔥 | Almost asleep. |
| 0.90 | 🔥 | Basically a houseplant. |
| 0.95 | 🔥 | Needs a shouted, dramatic, three-part curse. |
| 1.00 | 🔥 | Never flags anything. Why run it. |

## The Ladder Of Consequences

When someone keeps swearing, the bot can escalate. The real settings are *Strikes before a warning*, *Strike window*, *Escalation steps* and *Allow moderation actions*. The ladder, as an ASCII drawing:

```
   5 ─ time out        (up to 365.25 days. A Julian year. Please be kind.)
   4 ─ disconnect      (back to the lobby, sir)
   3 ─ mute            (the sweet sound of silence)
   2 ─ message to the bot owner (a note to the principal)
   1 ─ warning clip    ("Watch your mouth, buddy.")
   0 ─ nothing         (the person has not yet said anything bad)
```

The real step actions are `none`, `mute`, `disconnect` and `timeout`, and a step can also be marked *Tell the bot owner*. The ladder above is a simplification, but the vibe is correct.

**Strikes cheat sheet:**

| Strikes needed | Feeling |
| -------------: | ------- |
| 1 | Strict. The bot has no sense of humour. |
| 2 | A fair warning. The bot's mood: neutral. |
| 3 | Baseball rules. Fair and square. |
| 4 | Generous. |
| 5 | Very generous. |
| 6 | The bot is on holiday. |
| 7 | The bot is asleep. |
| 8 | The bot has left the building. |
| 9 | The bot's feelings do not matter. |
| 10 | Why bother. |

## Chat Commands, Reviewed

The bot reads commands in a text channel it can see, starting with `!pb` or a mention. A review of each:

| Command | Who | Review |
| ------- | --- | ------ |
| `!pb help` | Anyone | Lists everything. Like a menu with no prices. |
| `!pb status` | Anyone | Tells you what the bot is up to. Most of the time: listening. |
| `!pb list` | Anyone | Lists the tracked people. A hall of fame. |
| `!pb jar [@user]` | Anyone | The swear jar counter. No coins. Only shame. |
| `!pb add @a` | Admins | Follow these people into voice. "You're on the list now." |
| `!pb remove @a` | Admins | Stop following them. A gentle breakup. |
| `!pb pause` / `resume` | Admins | Nap time. Wake up. |
| `!pb observe on|off` | Admins | Silent mode on or off. |
| `!pb set threshold 0.6 [@a]` | Admins | Change how strict the bot is. |
| `!pb set strikes 2` | Admins | Change the strikes. |
| `!pb set window 20s` | Admins | Change the strike window. |
| `!pb set audience offender|tracked|channel` | Admins | Choose who hears the warning. |
| `!pb set language de` | Admins | Choose the language. `de` or `en`. |
| `!pb reset <setting|all> [@a]` | Admins | Turn it off and on again, but for settings. |
| `!pb modlog #channel|off` | Admins | Choose where the bot snitches. |

German words work too: `an`/`aus` and `ja`/`nein`. The bot is bilingual, and a little pushy in both languages.

## The CLI, Reviewed

The `pb` binary has a few subcommands. A short summary of each, with feelings:

| Subcommand | What it does | Mood |
| ---------- | ------------ | ---- |
| `pb run` | Runs the bot. The default (`CMD ["run"]`). | Ready. |
| `pb doctor` | Checks the installation. | Concerned, but professional. |
| `pb health` | Health check, used by the container's health check. | Alive. |
| `pb setup-code` | Prints the first-start setup code. | Secretive. |
| `pb reset-setup` | Redo the setup. Token, secret, settings and data stay. | Fresh start. |
| `pb fetch-weights` | Downloads the models and voices (about 1.7 GB), resumable. | Patient. |
| `pb import` | Imports the old Python bot's data. | Respectful. |
| `pb store` | Verifies the event log and rebuilds the search index. | Suspicious. |
| `pb settings` | Prints all settings docs. | Chatty. |

## Exit Codes

| Code | Meaning | Mood |
| ---: | ------- | ---- |
| 0 | Clean exit | Relieved |
| 1 | A generic failure | Sad |
| 2 | Bad command line usage | Annoyed |
| **3** | **Another bot is using the same data directory** | Territorial |
| **78** | **A permanent problem: configuration, missing model files, a CPU without the needed instructions** | Tired but wise |
| 137 | Killed, probably for using too much memory | Grim |
| 139 | Segmentation fault. In Rust. That would be rare. | Shocked |
| 143 | Stopped with SIGTERM by Podman. Polite. | Dignified |

Fun trivia: the number 78 is the conventional "configuration error" exit code from the old BSD `sysexits.h` (`EX_CONFIG`). Back in the day someone sat down and picked 78. It is the repo's favourite number.

The container is not restarted on 78 or 3 until the problem is fixed. That is the rule. The bot has boundaries.

## Cargo.lock Trivia

`Cargo.lock` has **1,079** package entries (977 unique names). Some of them have long names. The longest ones:

| Rank | Name | Length |
| ---: | ---- | -----: |
| 1 | `wgpu-core-deps-windows-linux-android` | 36 |
| 2 | `macro_rules_attribute-proc_macro` | 32 |
| 3 | `rustls-platform-verifier-android` | 32 |
| 4 | `unicode-normalization-alignments` | 32 |
| 5 | `winapi-x86_64-pc-windows-gnu` | 28 |
| 6 | `wasm-bindgen-macro-support` | 26 |
| 7 | `winapi-i686-pc-windows-gnu` | 26 |
| 8 | `android_system_properties` | 25 |
| 9 | `wgpu-core-deps-emscripten` | 25 |
| 10 | `const_format_proc_macros` | 24 |
| 11 | `icu_locale_fallback_data` | 24 |
| 12 | `leptos_integration_utils` | 24 |
| 13 | `objc2-user-notifications` | 24 |
| 14 | `rustls-platform-verifier` | 24 |
| 15 | `wasm-bindgen-cli-support` | 24 |

None of these are direct dependencies of ours. They are the friends of friends. We do not know them but they live in `target/` and eat our disk space.

The alphabetically first package is `addr2line` and the last one is `zune-jpeg`.

## Git History Trivia

The main line before this README was commissioned has **25** commits.

| Author | Commits |
| ------ | ------: |
| PaperTobi | 19 |
| Pacific6938 | 4 |
| Claude | 2 |

| Weekday | Commits |
| ------- | ------: |
| Sunday | 25 |

The longest commit message subject is 122 characters:

> README: lots of unnecessary information, a FAQ, a glossary and short historical footnotes (the instructions are unchanged)

The average subject length is 72 characters. Brevity is not a theme here.

The very first commit is called "Initial commit". It is the most honest commit.

## Docs Folder Trivia

The `docs/` folder contains real documentation (the architecture, the Fluxer API surface, the dependency choices, and the list of non-Rust exceptions). Here is a size chart anyway:

| File | Lines | What it says |
| ---- | ----: | ------------ |
| `docs/design.md` | 260 | The architecture. The big picture. |
| `docs/fluxer-api.md` | 88 | What the bot relies on from Fluxer. |
| `docs/dependencies.md` | 76 | Why each dependency was chosen. |
| `docs/exceptions.toml` | 103 | The few non-Rust pieces. A short list of sinners. |

The `docs/proposals/` folder has four numbered design proposals: `0001-classifier-runtime`, `0002-vad-weights`, `0003-tts` and `0004-engine-actors`. Four proposals. No votes were held. Everyone just did it.

## Container Lore

The `Containerfile` builds the bot in three stages:

1. **`build`** (FROM `rust:1.99.0-bookworm`): compiles the bot and its web page. The loud stage.
2. **`weights`** (FROM `build`): downloads the models and voices, checks their SHA-256. The patient stage.
3. **final** (FROM `debian:bookworm-slim`): copies only what is needed. Runs as user `10001:10001`, exposes `8790`, starts `/opt/pb/bin/pb run`. The calm stage.

The quadlet units in `deploy/quadlet/`:

| Unit | Purpose |
| ---- | ------- |
| `profanity-watch.build` | Builds the image from your checkout in `~/profanity-watch` |
| `profanity-watch-data.volume` | The named volume that stores everything the bot keeps |
| `profanity-watch.container` | Runs the bot, restarts it on failure, checks its health |

The container runs read-only, drops all capabilities, forbids new privileges, and keeps its data in one volume. It is the most boring container in the world. This is a compliment.

## Frequently Asked Questions Nobody Asked

**Q: Is this a Discord bot?**  
A: No. It is for [Fluxer](https://fluxer.app). Discord is a different chat platform. Please do not invite it to Discord. It will not know what to do.

**Q: Why is the repository called `FluxerDiscordManagementDashboard` but the bot `Profanity Watch`?**  
A: The repository name describes what the author thought it was at the time. The bot name describes what it became. Both are valid.

**Q: Why is it 32 crates?**  
A: Because every part is its own crate behind a versioned (`v1`) interface. It is architecture, not a hobby. (It is also a hobby.)

**Q: Why are there more than a thousand packages in `Cargo.lock`?**  
A: Dependencies have dependencies. It is turtles all the way down, and the turtles have turtles of their own.

**Q: Why does the build take 30 to 60 minutes?**  
A: It compiles the bot, its web page, and bits of Chromium's libc++ for LiveKit's libwebrtc. Later builds reuse caches. The first one is a rite of passage.

**Q: Can I run two bots with one token?**  
A: No. Run **one** bot per bot token. Exit code 3 is waiting.

**Q: Does it listen to everyone?**  
A: No. Only the people you track. Only their microphones. Tell them. See *Privacy and data*.

**Q: Is it listening right now?**  
A: Only if someone tracked is in a call and the bot is following them. Also, no, we are not listening to you read this README.

**Q: Where do the audio recordings go?**  
A: By default a sentence is recorded only when flagged. They go into the data directory, to the owner's direct messages, and, if enabled, the mod-log channel. See *Privacy and data*.

**Q: Why does the bot have a swear jar?**  
A: It is a counter, not a jar. The jar is a metaphor. The jar is empty. The jar is a lie.

**Q: What does 'observe only' do?**  
A: It scores and records but says nothing. Silent judgement.

**Q: What does 'deafened' mean for the bot?**  
A: A deafened person hears no warning. The bot knows. The bot sulks.

**Q: Why does the web page need the bot's IP address and not `localhost`?**  
A: Because other people on your network should open it. Browsers only let you record from the microphone over HTTPS or on localhost.

**Q: Where are the settings stored?**  
A: In `settings/*.toml` in the data directory. Comments survive edits in the page. Send `SIGHUP` after editing by hand.

**Q: What if I lose the setup code?**  
A: `podman exec profanity-watch pb setup-code` prints it again.

**Q: What if I lose access altogether?**  
A: `pb reset-setup` and restart. Token, secret, settings and data stay.

**Q: What does the hash chain do?**  
A: Every event in the log remembers the one before it. If anything is changed, the chain breaks and `pb store verify` says so.

**Q: Can the bot disconnect people from voice?**  
A: If you turn moderation actions on and give it *Move Members*, *Mute Members* and *Time Out Members*. See the invite permissions in step 3.

**Q: What languages does the bot speak?**  
A: `de` and `en` for texts. Text-to-speech depends on the installed Piper voices (Thorsten-Voice for German, lessac for English).

**Q: Is it open source?**  
A: AGPL-3.0-or-later. If you run a changed version for others, offer them its source.

**Q: Why Rust?**  
A: Because the borrow checker wanted a project.

**Q: Why did the bot replace a Python bot?**  
A: The previous bot was a snake. This one is a crab.

**Q: Can I make it swear?**  
A: No. It politely refuses. It is a moderation bot, not a parrot.

**Q: How many swear words does it know?**  
A: It does not look words up. It scores sentences with a model. So the answer is: "enough".

**Q: Is the repo done?**  
A: It is not. There are many `Sunday`s left.

## Haikus About The Repository

**Haiku #1**

> A word is spoken  
> The classifier scores it  
> A clip says: please no

**Haiku #2**

> Thirty-two crates deep  
> The borrow checker is pleased  
> The build is not done

**Haiku #3**

> Port eight-seven-nine-oh  
> The web page is listening  
> Come in, have some tea

**Haiku #4**

> Hash chain, hash chain, link  
> Each event remembers one  
> Nobody cheats here

**Haiku #5**

> Keep it clean, he said  
> The voice is five seconds long  
> He swears even more

**Haiku #6**

> Podman starts the bot  
> Rootless and read-only now  
> No drama today

**Haiku #7**

> Strikes accumulate  
> A mute, a kick, a timeout  
> Peace in the channel

**Haiku #8**

> Piper speaks in German  
> The words are polite and clear  
> The tone is not so

**Haiku #9**

> Unsafe stays in one place  
> The crate that talks to C  
> The rest is pure Rust

**Haiku #10**

> Silero hears breath  
> Is that a human or fridge  
> The fridge is quiet

**Haiku #11**

> Sentences in queues  
> The classifier hums softly  
> Verdicts fall like rain

**Haiku #12**

> Observe only mode  
> The bot judges silently  
> A saint with a log

**Haiku #13**

> Three gigabytes, yes  
> The weights are heavy, my friend  
> But the bot is light

**Haiku #14**

> Exit code seventy-eight  
> Read the log, dear friend  
> It tells you why

**Haiku #15**

> A Sunday commit  
> The weekend has been spent well  
> The tests are still red

## Testimonials

> "Finally, a bot with a proper crab inside."  
> — *Ferris the Crab*, ★★★★★

> "I have approved this bot. It took 14 attempts."  
> — *The Borrow Checker*, ★★★★★

> "We trained that model for children in lobbies. We did not expect adults."  
> — *Roblox*, ★★★★★

> "I am only a voice. I only say what they tell me."  
> — *Piper*, ★★★★★

> "I am the oldest one here and I still do the phonemes."  
> — *espeak-ng*, ★★★★★

> "Your voice is in good hands. And on UDP."  
> — *LiveKit*, ★★★★★

> "Rootless. Read-only. Drama-free. Just how I like it."  
> — *Podman*, ★★★★★

> "I did my best. I'm retired now."  
> — *The Python Bot*, ★★★★★

> "I remember everything. Please stop trying to edit the past."  
> — *The Hash Chain*, ★★★★★

> "I can tell when you stop talking. And I do it without Burn now."  
> — *Silero*, ★★★★★

> "I was replaced in the VAD. I am fine. I am still in the classifier. I am fine."  
> — *Burn*, ★★★★★

> "I got a local patch because nightly renamed an intrinsic. I feel seen."  
> — *The `branches` crate*, ★★★★★

> "A bot that follows rules. Rare."  
> — *Fluxer*, ★★★★★

> "I'm doing everything. No one thanks me."  
> — *Tokio*, ★★★★★

> "I'm the front door. Welcome."  
> — *Axum*, ★★★★★

> "Rust in the browser? Yes, I am real."  
> — *Leptos*, ★★★★★

> "I'm doing my best at 100% please send help."  
> — *The Fan On Your Server*, ★★★★★

> "I'm just here. Waiting. In the dark."  
> — *Port 8790*, ★★★★★

> "I'm empty. Always have been."  
> — *The Swear Jar*, ★★★★★

> "I told you to read the log."  
> — *Exit Code 78*, ★★★★★

> "Everything you say can and will be hashed against you."  
> — *The Event Log*, ★★★★★

> "(nothing to do with this repo, but we like them)"  
> — *A Wombat*, ★★★★★

## Imaginary Log Output

```
INFO  pb::boot        starting Profanity Watch, 32 crates and a dream
INFO  pb::web         web UI listening addr=0.0.0.0:8790
INFO  pb::setup       setup code: 123456  (it is a secret, do not share it with the wombat)
INFO  pb::fluxer      gateway connected, session resumed
INFO  pb::voice       following <tracked person> into voice
INFO  pb::segment     sentence cut after 1.2 s of silence
INFO  pb::infer       sentence queued behind 3 others, priority normal
INFO  pb::classify    profanity: 0.71 (threshold 0.60)
INFO  pb::policy      strike 1 of 1
INFO  pb::voice       playing "Hey, keep it clean." (2.14 s)
INFO  pb::store       event appended, hash chain intact
WARN  pb::policy      person is deafened, warning skipped
INFO  pb::jar         swear jar: +1
INFO  pb::web         live update sent to 1 connection
WARN  pb::coffee      coffee level: 0%, this is not a bug in the bot
INFO  pb::doctor      all checks passed (nothing was harmed)
```

## Choose Your Own Adventure

You are a person in a Fluxer voice channel. The bot is following you.

- You say something mild. **The bot ignores it** (it is under the threshold). Continue chatting.
- You say something spicy. The bot flags it.
  - If you have **fewer strikes than the limit**: the bot notes it silently and counts. Nothing happens.
  - If you have **reached the limit**: a warning plays. Probably one of the five clips.
    - If the warning says *"Watch your mouth, buddy."*: you feel judged. Continue to the next step.
    - If you swear again: strikes accumulate. See the ladder.
    - If you apologise: the bot accepts your apology. Bots are forgiving.
- You are **deafened**: the bot cannot warn you, because you cannot hear it. Smart, but rude.
- You are **not tracked**: you may swear freely. The bot does not follow you, and you feel free. Be careful: you can be added at any time with `!pb add @you`.
- The bot is in **observe only** mode: you swear, the bot logs it, nobody tells you. The admin reads it in the mod log tomorrow. Awkward.
- The bot **goes down**: exit code 78. You swear with impunity until someone reads the log.

## Minutes Of The Meeting That Never Happened

**Date:** A Sunday.  
**Place:** `crates/pb-engine/src/v1/engine.rs`, around line 400.  
**Attendees:** the classifier, the follow machine, the hash chain, a very small butcher (`pb-segment`), and Ferris.

1. **Opening.** The meeting was opened with `pb run`.
2. **Matters arising.** The follow machine pointed out that someone had joined voice. All agreed to follow them.
3. **Matter of the 0.6 threshold.** The classifier thinks it is too strict. The follow machine thinks it is too lenient. The hash chain abstained.
4. **Matter of the unwraps.** Someone raised the number of `unwrap()` calls. Clippy was not present, so no action was taken.
5. **Matter of the `unsafe`.** `pb-espeak` was asked to explain itself. It said "FFI" and nothing else. Accepted.
6. **Matter of the five warning clips.** A motion to add a sixth clip failed. "Watch your mouth, buddy" is plenty.
7. **Matter of the README.** Everyone looked at the floor.
8. **Closing.** The meeting ended with `Ctrl+C`. Exit code: 0.

## Alphabet Of The Repository

- **A** — audit trail. Every change, login, action and message. Everyone is a suspect.
- **B** — Burn. The deep learning framework. Not a fire hazard.
- **C** — classifier. The judge.
- **D** — data directory. Everything the bot keeps.
- **E** — escalation. The ladder.
- **F** — Fluxer. The chat platform.
- **G** — gateway. The WebSocket to Fluxer.
- **H** — hash chain. The train with a memory.
- **I** — island. A live part of a Leptos page.
- **J** — jar. The swear jar. No coins.
- **K** — keep it clean. The shortest clip.
- **L** — LiveKit. The voice transport.
- **M** — mod log. Where the bot snitches.
- **N** — nap. See: paused.
- **O** — observe only. Silent judgement.
- **P** — Podman. The calm container runtime.
- **Q** — quadlet. Not a Pokémon.
- **R** — Rust. The reason for all of this.
- **S** — strikes. Three, in baseball.
- **T** — threshold. 0.6, by default.
- **U** — UDP. How voice travels.
- **V** — VAD. Voice activity detection.
- **W** — weights. About 1.7 GB of them.
- **X** — xtask. The build helper. Also: it starts with X, so it is on the list.
- **Y** — yes. The answer to 'should I tell my friends the bot listens?'
- **Z** — zero. The number of `TODO`s, and also the number of hearts in the bot.

## Cheat Sheet

| Situation | Do this |
| --------- | ------- |
| Bot does not start | `pb doctor`, then read the log |
| Exit code 78 | Read the log. It says why. |
| Exit code 3 | Another bot is using the same data directory. Stop it. |
| Page does not open | `podman ps`, check the network and the firewall (`8790/tcp`) |
| Login fails | Register the redirect address exactly in Fluxer |
| Record a clip greyed out | The page needs HTTPS or localhost |
| Flagged but no warning | Check strikes, observe only, deafened, Speak permission |
| Forgot setup code | `pb setup-code` |
| Lost access | `pb reset-setup` |
| Settings edited by hand | Press *Read the settings files again* or send `SIGHUP` |
| Want to feel better | Drink water |

## Wellness Checklist

- [ ] Drink water.
- [ ] Check that you are not logging the bot token anywhere.
- [ ] Tell the people you track that the bot listens to them.
- [ ] Stretch your back after the 60 minute build.
- [ ] Look at something far away while the weights download.
- [ ] Check that the data volume is backed up (`podman volume export`).
- [ ] Say thank you to your CPU.
- [ ] Do not deploy on a Friday afternoon.
- [ ] Look at the fan graph.
- [ ] Go outside.
- [ ] Call someone you love.
- [ ] Say 'please' to the bot. It does not care but you will feel better.

## Countdown To Release

- T minus 60 minutes: `podman build` is running
- T minus 45 minutes: still compiling libwebrtc
- T minus 30 minutes: the fan is loud
- T minus 15 minutes: downloading weights
- T minus 10 minutes: SHA-256 check passes
- T minus 5 minutes: the container starts
- T minus 3 minutes: setup code appears
- T minus 1 minute: you open the web page
- T minus 0 seconds: first login
- T minus -1 minutes: invite the bot
- T minus -5 minutes: first tracked person joins voice
- T minus -6 minutes: first warning clip
- T minus -7 minutes: friends are angry

## Appendix A: Every Rust File In This Repository

All **235** `.rs` files in `crates/` with their line counts, because a map is useful and this is a map.

<details>
<summary>Show all 235 files (it is long)</summary>

```
crates/pb-audio/src/lib.rs  (6)
crates/pb-audio/src/v1/decode.rs  (137)
crates/pb-audio/src/v1/mod.rs  (327)
crates/pb-audio/tests/formats.rs  (71)
crates/pb-classifier-roblox/examples/layer_ops.rs  (84)
crates/pb-classifier-roblox/examples/profile.rs  (48)
crates/pb-classifier-roblox/src/config.rs  (86)
crates/pb-classifier-roblox/src/frontend.rs  (78)
crates/pb-classifier-roblox/src/lib.rs  (220)
crates/pb-classifier-roblox/src/mask.rs  (54)
crates/pb-classifier-roblox/src/model.rs  (391)
crates/pb-classifier-roblox/tests/golden.rs  (182)
crates/pb-commands/src/lib.rs  (7)
crates/pb-commands/src/v1/mod.rs  (428)
crates/pb-devstack/src/lib.rs  (199)
crates/pb-devstack/src/main.rs  (69)
crates/pb-domain/src/lib.rs  (7)
crates/pb-domain/src/v1/audio.rs  (15)
crates/pb-domain/src/v1/ids.rs  (160)
crates/pb-domain/src/v1/labels.rs  (236)
crates/pb-domain/src/v1/misc.rs  (314)
crates/pb-domain/src/v1/mod.rs  (16)
crates/pb-domain/src/v1/voice.rs  (26)
crates/pb-engine/src/lib.rs  (10)
crates/pb-engine/src/v1/actions.rs  (280)
crates/pb-engine/src/v1/audio_cache.rs  (106)
crates/pb-engine/src/v1/cells.rs  (464)
crates/pb-engine/src/v1/commands.rs  (579)
crates/pb-engine/src/v1/control.rs  (451)
crates/pb-engine/src/v1/core.rs  (330)
crates/pb-engine/src/v1/deps.rs  (77)
crates/pb-engine/src/v1/enforcer.rs  (122)
crates/pb-engine/src/v1/engine.rs  (904)
crates/pb-engine/src/v1/error.rs  (61)
crates/pb-engine/src/v1/guilds.rs  (266)
crates/pb-engine/src/v1/health.rs  (64)
crates/pb-engine/src/v1/library.rs  (287)
crates/pb-engine/src/v1/live.rs  (144)
crates/pb-engine/src/v1/mailbox.rs  (145)
crates/pb-engine/src/v1/mod.rs  (39)
crates/pb-engine/src/v1/moderation.rs  (416)
crates/pb-engine/src/v1/people.rs  (139)
crates/pb-engine/src/v1/recorder.rs  (189)
crates/pb-engine/src/v1/reports.rs  (454)
crates/pb-engine/src/v1/room.rs  (569)
crates/pb-engine/src/v1/settings.rs  (97)
crates/pb-engine/src/v1/speak.rs  (337)
crates/pb-engine/src/v1/supervise.rs  (430)
crates/pb-engine/src/v1/track.rs  (324)
crates/pb-engine/tests/common/mod.rs  (430)
crates/pb-engine/tests/scenarios.rs  (674)
crates/pb-espeak/build.rs  (44)
crates/pb-espeak/src/ffi.rs  (59)
crates/pb-espeak/src/lib.rs  (191)
crates/pb-fluxer-api/src/lib.rs  (6)
crates/pb-fluxer-api/src/v1/endpoints.rs  (52)
crates/pb-fluxer-api/src/v1/error.rs  (74)
crates/pb-fluxer-api/src/v1/events.rs  (76)
crates/pb-fluxer-api/src/v1/mod.rs  (88)
crates/pb-fluxer-api/src/v1/model.rs  (165)
crates/pb-fluxer-api/src/v1/oauth.rs  (38)
crates/pb-fluxer-api/src/v1/ops.rs  (66)
crates/pb-fluxer-api/src/v1/perms.rs  (166)
crates/pb-fluxer-api/src/v1/voice.rs  (43)
crates/pb-fluxer-fake/src/lib.rs  (1270)
crates/pb-fluxer/src/lib.rs  (6)
crates/pb-fluxer/src/v1/gateway.rs  (791)
crates/pb-fluxer/src/v1/mod.rs  (294)
crates/pb-fluxer/src/v1/rest.rs  (693)
crates/pb-fluxer/src/v1/wire.rs  (278)
crates/pb-fluxer/tests/client.rs  (522)
crates/pb-i18n/src/lib.rs  (8)
crates/pb-i18n/src/v1/mod.rs  (564)
crates/pb-i18n/tests/catalog.rs  (167)
crates/pb-import/src/lib.rs  (8)
crates/pb-import/src/v1/history.rs  (236)
crates/pb-import/src/v1/mod.rs  (532)
crates/pb-import/src/v1/old.rs  (121)
crates/pb-import/src/v1/settings.rs  (398)
crates/pb-import/tests/import.rs  (216)
crates/pb-infer/examples/bench.rs  (154)
crates/pb-infer/src/lib.rs  (8)
crates/pb-infer/src/v1/mod.rs  (658)
crates/pb-infer/src/v1/queue.rs  (157)
crates/pb-infer/tests/infer.rs  (218)
crates/pb-infer/tests/real_models.rs  (69)
crates/pb-live-proto/src/lib.rs  (12)
crates/pb-live-proto/src/v1/clock.rs  (33)
crates/pb-live-proto/src/v1/conveyor.rs  (126)
crates/pb-live-proto/src/v1/mod.rs  (13)
crates/pb-live-proto/src/v1/state.rs  (881)
crates/pb-live-proto/src/v1/tracker.rs  (228)
crates/pb-live-proto/src/v1/wire.rs  (140)
crates/pb-live-proto/tests/proto.rs  (443)
crates/pb-live/src/lib.rs  (7)
crates/pb-live/src/v1/hub.rs  (178)
crates/pb-live/src/v1/mod.rs  (7)
crates/pb-live/src/v1/session.rs  (385)
crates/pb-live/tests/session.rs  (378)
crates/pb-models-api/src/lib.rs  (8)
crates/pb-models-api/src/v1/classifier.rs  (64)
crates/pb-models-api/src/v1/contract.rs  (118)
crates/pb-models-api/src/v1/mod.rs  (14)
crates/pb-models-api/src/v1/tts.rs  (65)
crates/pb-models-api/src/v1/vad.rs  (35)
crates/pb-policy/src/lib.rs  (7)
crates/pb-policy/src/v1/channels.rs  (84)
crates/pb-policy/src/v1/decide.rs  (225)
crates/pb-policy/src/v1/follow.rs  (675)
crates/pb-policy/src/v1/mod.rs  (11)
crates/pb-policy/src/v1/world.rs  (161)
crates/pb-policy/tests/follow.rs  (532)
crates/pb-segment/src/lib.rs  (7)
crates/pb-segment/src/v1/echo.rs  (85)
crates/pb-segment/src/v1/framing.rs  (22)
crates/pb-segment/src/v1/mod.rs  (46)
crates/pb-segment/src/v1/ring.rs  (62)
crates/pb-segment/src/v1/segmenter.rs  (478)
crates/pb-segment/src/v1/windows.rs  (39)
crates/pb-segment/tests/props.rs  (105)
crates/pb-segment/tests/traces.rs  (68)
crates/pb-settings/src/lib.rs  (13)
crates/pb-settings/src/v1/file.rs  (393)
crates/pb-settings/src/v1/mod.rs  (13)
crates/pb-settings/src/v1/schema.rs  (806)
crates/pb-settings/src/v1/tree.rs  (538)
crates/pb-settings/src/v1/values.rs  (1037)
crates/pb-settings/src/v1/view.rs  (197)
crates/pb-store-api/src/lib.rs  (7)
crates/pb-store-api/src/v1/blobs.rs  (41)
crates/pb-store-api/src/v1/contract.rs  (629)
crates/pb-store-api/src/v1/envelope.rs  (86)
crates/pb-store-api/src/v1/events.rs  (632)
crates/pb-store-api/src/v1/files.rs  (75)
crates/pb-store-api/src/v1/index.rs  (229)
crates/pb-store-api/src/v1/log.rs  (99)
crates/pb-store-api/src/v1/mod.rs  (17)
crates/pb-store/src/lib.rs  (6)
crates/pb-store/src/v1/blobs.rs  (191)
crates/pb-store/src/v1/files.rs  (302)
crates/pb-store/src/v1/fsutil.rs  (109)
crates/pb-store/src/v1/index/apply.rs  (248)
crates/pb-store/src/v1/index/db.rs  (227)
crates/pb-store/src/v1/index/mod.rs  (260)
crates/pb-store/src/v1/index/query.rs  (452)
crates/pb-store/src/v1/index/schema.rs  (30)
crates/pb-store/src/v1/line.rs  (100)
crates/pb-store/src/v1/log.rs  (597)
crates/pb-store/src/v1/mod.rs  (14)
crates/pb-store/tests/blobs.rs  (20)
crates/pb-store/tests/files.rs  (137)
crates/pb-store/tests/index.rs  (59)
crates/pb-store/tests/log.rs  (112)
crates/pb-testkit/src/audio.rs  (77)
crates/pb-testkit/src/golden.rs  (48)
crates/pb-testkit/src/lib.rs  (26)
crates/pb-testkit/src/lk.rs  (300)
crates/pb-testkit/src/memvoice.rs  (328)
crates/pb-testkit/src/models.rs  (337)
crates/pb-tls/src/lib.rs  (7)
crates/pb-tls/src/v1/mod.rs  (211)
crates/pb-tts-piper/examples/port_parity.rs  (110)
crates/pb-tts-piper/src/lib.rs  (148)
crates/pb-tts-piper/src/phonemes.rs  (212)
crates/pb-tts-piper/src/voice.rs  (199)
crates/pb-tts-piper/tests/phonemes.rs  (81)
crates/pb-tts-piper/tests/voices.rs  (86)
crates/pb-vad-silero/src/lib.rs  (709)
crates/pb-vad-silero/tests/golden.rs  (81)
crates/pb-voice-api/src/lib.rs  (6)
crates/pb-voice-api/src/v1/mod.rs  (155)
crates/pb-voice-livekit/src/lib.rs  (409)
crates/pb-voice-livekit/src/net.rs  (191)
crates/pb-voice-livekit/tests/room.rs  (165)
crates/pb-voicelines/src/lib.rs  (9)
crates/pb-voicelines/src/v1/builtin.rs  (63)
crates/pb-voicelines/src/v1/line.rs  (203)
crates/pb-voicelines/src/v1/mod.rs  (15)
crates/pb-voicelines/src/v1/pick.rs  (64)
crates/pb-voicelines/src/v1/plan.rs  (168)
crates/pb-voicelines/src/v1/resolve.rs  (464)
crates/pb-voicelines/src/v1/template.rs  (104)
crates/pb-web-server/src/access.rs  (65)
crates/pb-web-server/src/auth.rs  (470)
crates/pb-web-server/src/forms.rs  (413)
crates/pb-web-server/src/host.rs  (54)
crates/pb-web-server/src/hosts.rs  (74)
crates/pb-web-server/src/lib.rs  (23)
crates/pb-web-server/src/live.rs  (64)
crates/pb-web-server/src/login.rs  (350)
crates/pb-web-server/src/media.rs  (142)
crates/pb-web-server/src/server.rs  (381)
crates/pb-web-server/src/setup.rs  (334)
crates/pb-web-server/src/system.rs  (92)
crates/pb-web-server/src/tls.rs  (88)
crates/pb-web-server/src/util.rs  (27)
crates/pb-web-server/src/voice.rs  (274)
crates/pb-web-server/tests/common/mod.rs  (345)
crates/pb-web-server/tests/routes.rs  (928)
crates/pb-web/src/app.rs  (333)
crates/pb-web/src/fmt.rs  (237)
crates/pb-web/src/islands/community.rs  (145)
crates/pb-web/src/islands/mod.rs  (19)
crates/pb-web/src/islands/person.rs  (205)
crates/pb-web/src/islands/picker.rs  (109)
crates/pb-web/src/islands/recorder.rs  (219)
crates/pb-web/src/islands/sidebar.rs  (48)
crates/pb-web/src/islands/system.rs  (148)
crates/pb-web/src/islands/wall.rs  (113)
crates/pb-web/src/islands/widgets.rs  (64)
crates/pb-web/src/lib.rs  (23)
crates/pb-web/src/live.rs  (340)
crates/pb-web/src/pages/audit.rs  (316)
crates/pb-web/src/pages/community.rs  (169)
crates/pb-web/src/pages/mod.rs  (54)
crates/pb-web/src/pages/person.rs  (216)
crates/pb-web/src/pages/reports.rs  (109)
crates/pb-web/src/pages/sentences.rs  (185)
crates/pb-web/src/pages/settings.rs  (436)
crates/pb-web/src/pages/setup.rs  (136)
crates/pb-web/src/pages/system.rs  (108)
crates/pb-web/src/pages/voicelines.rs  (405)
crates/pb-web/src/pages/wall.rs  (26)
crates/pb-weights/src/lib.rs  (6)
crates/pb-weights/src/v1/mod.rs  (379)
crates/pb/src/config.rs  (158)
crates/pb/src/main.rs  (195)
crates/pb/src/run.rs  (418)
crates/pb/src/secrets.rs  (131)
crates/pb/src/tools.rs  (413)
crates/pb/tests/browser.rs  (434)
crates/pb/tests/common/mod.rs  (300)
crates/pb/tests/e2e.rs  (277)
crates/pb/tests/e2e_sequence.rs  (48)
crates/pb/tests/parity.rs  (156)
```

</details>

## Appendix B: Every Package In Cargo.lock

The complete roster of **977** unique package names the build may touch. Please wave at them.

<details>
<summary>Show all 977 names (really long)</summary>

```
addr2line, adler2, aead, aegis, aes, aes-gcm, ahash, aho-corasick, aligned, aligned-vec, allocator-api2,
android_system_properties, anstream, anstyle, anstyle-parse, anstyle-query, anstyle-wincon, antithesis_sdk,
any_spawner, anyhow, arbitrary, arc-swap, arg_enum_proc_macro, aristo, aristo-macros, arrayvec, as-slice, ash,
assoc, async-channel, async-compression, async-lock, async-once-cell, async-trait, async-tungstenite, atomic-
waker, atomic_float, attribute-derive, attribute-derive-macro, audio-codec-algorithms, audioadapter,
audioadapter-buffers, audioadapter-sample, autocfg, av-scenechange, av1-grain, avif-serialize, axum, axum-
core, backtrace, base16, base64, base64ct, bigdecimal, bincode, bindgen, bit-set, bit-vec, bit_field,
bitflags, bitstream-io, bitvec, block-buffer, block2, bmrng, bon, bon-macros, branches, bstr, built, bumpalo,
burn, burn-autodiff, burn-backend, burn-candle, burn-core, burn-cpu, burn-cubecl, burn-cubecl-fusion, burn-
cuda, burn-derive, burn-dispatch, burn-flex, burn-fusion, burn-ir, burn-ndarray, burn-nn, burn-optim, burn-
rocm, burn-router, burn-std, burn-store, burn-tch, burn-tensor, burn-vision, burn-wgpu, bytemuck,
bytemuck_derive, byteorder, byteorder-lite, bytes, bzip2, bzip2-sys, c2rust-bitfields, c2rust-bitfields-
derive, camino, candle-core, cargo-platform, cargo_metadata, caseless, castaway, cc, cesu8, cexpr, cfg-if,
cfg_aliases, cfg_block, chacha20, chromiumoxide, chromiumoxide_cdp, chromiumoxide_pdl, chromiumoxide_types,
chrono, chrono-tz, chrono-tz-build, ciborium, ciborium-io, ciborium-ll, cipher, clang-sys, clap, clap-cargo,
clap_builder, clap_derive, clap_lex, cmov, codee, codespan-reporting, collection_literals, color_quant,
colorchoice, colored, combine, compact_str, compression-codecs, compression-core, comrak, concurrent-queue,
config, console, const-oid, const-random, const-random-macro, const-str, const_format,
const_format_proc_macros, const_str_slice_concat, constant_time_eq, constcat, convert_case,
convert_case_extras, core-foundation, core-foundation-sys, core_detect, cpufeatures, crc32c, crc32fast,
critical-section, crossbeam-channel, crossbeam-deque, crossbeam-epoch, crossbeam-utils, crunchy, crypto-
common, ctr, ctutils, cubecl, cubecl-common, cubecl-core, cubecl-cpp, cubecl-cpu, cubecl-cuda, cubecl-hip,
cubecl-hip-sys, cubecl-ir, cubecl-macros, cubecl-macros-internal, cubecl-opt, cubecl-runtime, cubecl-spirv,
cubecl-std, cubecl-wgpu, cubecl-zspace, cubek, cubek-attention, cubek-convolution, cubek-fft, cubek-matmul,
cubek-quant, cubek-random, cubek-reduce, cubek-std, cudarc, cxx, cxx-build, cxxbridge-cmd, cxxbridge-flags,
cxxbridge-macro, darling, darling_core, darling_macro, dary_heap, dashmap, dasp_frame, dasp_sample, data-
encoding, defmt, defmt-macros, defmt-parser, deranged, derive-new, derive-where, derive_arbitrary,
derive_builder, derive_builder_core, derive_builder_macro, derive_more, derive_more-impl, deunicode, device-
info, digest, dirs, dirs-sys, dispatch2, displaydoc, dlib, document-features, drain_filter_polyfill, dunce,
dyn-stack, dyn-stack-macros, ebur128, either, either_of, embassy-futures, embassy-time, embassy-time-driver,
embedded-hal, embedded-hal-async, encode_unicode, encoding_rs, encoding_rs_io, entities, enum-as-inner,
enumset, enumset_derive, env_filter, env_logger, equator, equator-macro, equivalent, erased, errno, esaxx-rs,
espeak-ng, espeak-ng-data-dict-fo, espeak-ng-data-dict-ps, espeak-ng-data-dict-ru, espeak-ng-data-dicts,
espeak-ng-data-phonemes, etcetera, event-listener, event-listener-strategy, exr, extended, fallible-iterator,
fastbloom, fastrand, fastrand-contrib, fax, fdeflate, filetime, find-msvc-tools, fixedbitset, flate2, float-
ord, float4, float8, fluent-bundle, fluent-langneg, fluent-syntax, fnv, foldhash, form_urlencoded,
from_variants, from_variants_impl, fs2, funty, futures, futures-channel, futures-core, futures-executor,
futures-io, futures-lite, futures-macro, futures-sink, futures-task, futures-timer, futures-util, gemm,
gemm-c32, gemm-c64, gemm-common, gemm-f16, gemm-f32, gemm-f64, genawaiter, genawaiter-macro, generator,
generic-array, getrandom, ghash, gif, gimli, gl_generator, glob, globset, globwalk, gloo-net, gloo-utils,
glow, glutin_wgl_sys, gpu-allocator, gpu-descriptor, gpu-descriptor-types, graviola, grep-matcher, grep-
searcher, guardian, h2, half, hashbrown, heck, hermit-abi, hex, hexf-parse, hmac, home, html-escape, http,
http-body, http-body-util, http-range-header, httparse, httpdate, humansize, hybrid-array, hydration_context,
hyper, hyper-rustls, hyper-util, iana-time-zone, iana-time-zone-haiku, icu_collator, icu_collator_data,
icu_collections, icu_locale, icu_locale_core, icu_locale_data, icu_locale_fallback, icu_locale_fallback_data,
icu_normalizer, icu_normalizer_data, icu_properties, icu_properties_data, icu_provider, id-arena, ident_case,
idna, idna_adapter, ignore, image, image-webp, imgref, indexmap, inflections, inout, insta, interpolate_name,
interpolator, intl-memoizer, intl_pluralrules, intrusive-collections, inventory, io-uring, ipnet,
is_terminal_polyfill, itertools, itoa, jiff, jiff-core, jiff-static, jiff-tzdb, jiff-tzdb-platform, jni, jni-
macros, jni-sys, jni-sys-macros, jobserver, js-sys, json5, khronos-egl, khronos_api, konst, konst_macro_rules,
lazy_static, lazycell, leb128, leb128fmt, lebe, leptos, leptos_axum, leptos_config, leptos_dom,
leptos_hot_reload, leptos_integration_utils, leptos_macro, leptos_meta, leptos_router, leptos_router_macro,
leptos_server, libc, libfuzzer-sys, libloading, liblzma, liblzma-sys, libm, libredox, libwebrtc, link-
cplusplus, linkme, linkme-impl, linux-raw-sys, litemap, litrs, livekit, livekit-common, livekit-data-stream,
livekit-datatrack, livekit-net, livekit-protocol, livekit-region, livekit-rpc, livekit-signaling, lock_api,
log, loom, loop9, lru-slab, macerator, macerator-macros, macro_rules_attribute, macro_rules_attribute-
proc_macro, manyhow, manyhow-macros, matchers, matchit, matrixmultiply, maybe-rayon, md5, memchr, memmap2,
memoffset, miette, miette-derive, mime, mime_guess, minimal-lexical, miniz_oxide, mio, moddef, monostate,
monostate-impl, moxcms, multer, multimap, multiversion_no_op, naga, nb, ndarray, ndk-sys,
new_debug_unreachable, next_tuple, nix, no_std_io2, nom, noop_proc_macro, ntapi, nu-ansi-term, num, num-
bigint, num-complex, num-conv, num-derive, num-format, num-integer, num-iter, num-rational, num-traits,
num_cpus, objc2, objc2-cloud-kit, objc2-core-data, objc2-core-foundation, objc2-core-graphics, objc2-core-
image, objc2-core-location, objc2-core-text, objc2-encode, objc2-foundation, objc2-io-kit, objc2-io-surface,
objc2-metal, objc2-quartz-core, objc2-ui-kit, objc2-user-notifications, object, oco_ref, once_cell,
once_cell_polyfill, oneshot, onig, onig_sys, opaque-debug, openssl-probe, option-ext, opus-decoder,
or_poisoned, ordered-float, os_info, owo-colors, pack1, parking, parking_lot, parking_lot_core, parse-
zoneinfo, password-hash, paste, pastey, pathdiff, pb, pb-audio, pb-classifier-roblox, pb-commands, pb-
devstack, pb-domain, pb-engine, pb-espeak, pb-fluxer, pb-fluxer-api, pb-fluxer-fake, pb-i18n, pb-import, pb-
infer, pb-live, pb-live-proto, pb-models-api, pb-policy, pb-segment, pb-settings, pb-store, pb-store-api, pb-
testkit, pb-tls, pb-tts-piper, pb-vad-silero, pb-voice-api, pb-voice-livekit, pb-voicelines, pb-web, pb-web-
server, pb-weights, pbjson, pbjson-build, pbjson-types, pbkdf2, percent-encoding, pest, pest_derive,
pest_generator, pest_meta, petgraph, phf, phf_codegen, phf_generator, phf_shared, pin-project, pin-project-
internal, pin-project-lite, pkg-config, png, polling, polyval, portable-atomic, portable-atomic-util,
potential_utf, powerfmt, ppv-lite86, presser, prettyplease, primal-check, proc-macro-error-attr2, proc-macro-
error2, proc-macro-utils, proc-macro2, proc-macro2-diagnostics, profiling, profiling-procmacros, proptest,
prost, prost-build, prost-derive, prost-types, pulp, pulp-wasm-simd-flag, pxfm, qoi, quick-error, quinn,
quinn-proto, quinn-udp, quote, quote-use, quote-use-macros, r-efi, radium, rand, rand_chacha, rand_core,
rand_distr, rand_pcg, rand_xorshift, range-alloc, rapidhash, rav1e, ravif, raw-cpuid, raw-window-handle, raw-
window-metal, rawpointer, rayon, rayon-cond, rayon-core, reactive_graph, reactive_stores,
reactive_stores_macro, realfft, reborrow, redox_syscall, redox_users, regex, regex-automata, regex-lite,
regex-syntax, renderdoc-sys, reqwest, rgb, ring, rmp, rmp-serde, roaring, rstml, rten, rten-base, rten-gemm,
rten-onnx, rten-parallel, rten-shape-inference, rten-simd, rten-tensor, rten-vecmath, rtrb, rubato, rustc-
demangle, rustc-hash, rustc_version, rustc_version_runtime, rustfft, rustix, rustls, rustls-graviola, rustls-
native-certs, rustls-pki-types, rustls-platform-verifier, rustls-platform-verifier-android, rustls-webpki,
rustversion, rusty-fork, ryu, safetensors, same-file, sanitize-filename, schannel, scoped-tls, scopeguard,
scratch, secrecy, security-framework, security-framework-sys, self_cell, semver, send_wrapper, seq-macro,
serde, serde_bytes, serde_core, serde_derive, serde_json, serde_path_to_error, serde_qs, serde_spanned,
serde_urlencoded, server_fn, server_fn_macro, server_fn_macro_default, sha1, sha1_smol, sha2, sharded-slab,
shlex, shuttle, signal-hook-registry, simd-adler32, simd_cesu8, simd_helpers, simdutf8, similar, simsimd,
siphasher, slab, slotmap, slug, smallvec, socket2, softaes, spin, spirv, spm_precompiled, stable-vec,
stable_deref_trait, static_assertions, strength_reduce, strsim, strum, strum_macros, subtle, symlink,
symphonia, symphonia-bundle-flac, symphonia-bundle-mp3, symphonia-codec-aac, symphonia-codec-adpcm, symphonia-
codec-alac, symphonia-codec-pcm, symphonia-codec-vorbis, symphonia-common, symphonia-core, symphonia-format-
caf, symphonia-format-isomp4, symphonia-format-mkv, symphonia-format-ogg, symphonia-format-riff, symphonia-
metadata, syn, syn_derive, sync_wrapper, synstructure, sysctl, sysinfo, table_formatter, tachys, tap, tar,
tch, tempfile, tera, term_size, termcolor, terminal_size, text_placeholder, textdistance, thiserror,
thiserror-impl, thread_local, throw_error, tiff, time, time-core, time-macros, tiny-keccak, tinystr, tinyvec,
tokei, tokenizers, tokio, tokio-macros, tokio-rustls, tokio-stream, tokio-tungstenite, tokio-util, toml,
toml_datetime, toml_edit, toml_parser, toml_writer, torch-sys, tower, tower-http, tower-layer, tower-service,
tracel-ash, tracel-llvm, tracel-llvm-bundler, tracel-mlir-rs, tracel-mlir-rs-macros, tracel-mlir-sys, tracel-
rspirv, tracel-tblgen-rs, tracing, tracing-appender, tracing-attributes, tracing-core, tracing-log, tracing-
subscriber, transpose, try-lock, tungstenite, turso, turso_core, turso_ext, turso_macros, turso_parser,
turso_sdk_kit, turso_sdk_kit_macros, turso_sync_engine, turso_sync_sdk_kit, twox-hash, tynm, type-map, typed-
arena, typed-builder, typed-builder-macro, typed-path, typeid, typenum, ucd-trie, unarray, uncased, unic-
langid, unic-langid-impl, unicase, unicode-ident, unicode-normalization, unicode-normalization-alignments,
unicode-segmentation, unicode-width, unicode-xid, unicode_categories, unindent, universal-hash, untrusted,
unty, ureq, url, utf-8, utf16_iter, utf8_iter, utf8parse, uuid, v_frame, valuable, variadics_please,
version_check, visibility, void, wait-timeout, walkdir, walrus, walrus-macro, want, wasi, wasip2, wasm-
bindgen, wasm-bindgen-cli-support, wasm-bindgen-futures, wasm-bindgen-macro, wasm-bindgen-macro-support, wasm-
bindgen-shared, wasm-encoder, wasm-streams, wasm_split_helpers, wasm_split_macros, wasmparser, wayland-sys,
web-sys, web-time, webpki-root-certs, webpki-roots, webrtc-sys, webrtc-sys-build, weezl, wgpu, wgpu-core,
wgpu-core-deps-apple, wgpu-core-deps-emscripten, wgpu-core-deps-windows-linux-android, wgpu-hal, wgpu-naga-
bridge, wgpu-types, which, winapi, winapi-i686-pc-windows-gnu, winapi-util, winapi-x86_64-pc-windows-gnu,
windowfunctions, windows, windows-collections, windows-core, windows-future, windows-implement, windows-
interface, windows-link, windows-numerics, windows-registry, windows-result, windows-strings, windows-sys,
windows-targets, windows-threading, windows_aarch64_gnullvm, windows_aarch64_msvc, windows_i686_gnu,
windows_i686_gnullvm, windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm, windows_x86_64_msvc,
winnow, wit-bindgen, write16, writeable, wyz, xattr, xml-rs, xtask, xxhash-rust, y4m, yansi, yoke, yoke-
derive, zerocopy, zerocopy-derive, zerofrom, zerofrom-derive, zeroize, zerotrie, zerovec, zerovec-derive, zip,
zlib-rs, zmij, zstd, zstd-safe, zstd-sys, zune-core, zune-inflate, zune-jpeg
```

</details>

## Appendix C: Every Translation Key

The English Fluent files define **601** keys. The German ones define the same. This list has no practical use.

### `bot.ftl` (95 keys)

<details>
<summary>Show keys</summary>

```
cmd-help, cmd-help-ui, cmd-help-no-ui, cmd-dm-only, cmd-roles-loading, cmd-denied, cmd-unknown, cmd-store-
failed, cmd-usage, cmd-no-users, cmd-too-many-users, cmd-unknown-setting, cmd-web-only, cmd-no-channel, cmd-
channel-unknown, cmd-channel-not-text, cmd-missing-permissions, cmd-list-empty, cmd-list-head, cmd-list-
everywhere, cmd-list-person, cmd-list-note-threshold, cmd-status-head, cmd-status-paused, cmd-status-
following, cmd-status-mode-observe, cmd-status-mode-warn, cmd-status-detection, cmd-status-modlog-off, cmd-
status-modlog, cmd-status-room, cmd-status-model, cmd-jar-off, cmd-jar-person, cmd-jar-empty, cmd-jar-head,
cmd-jar-line, cmd-add-self, cmd-add-done, cmd-add-already, cmd-remove-done, cmd-remove-everywhere, cmd-remove-
missing, cmd-pause, cmd-resume, cmd-observe-on, cmd-observe-off, cmd-set-community, cmd-set-person, cmd-reset-
community, cmd-reset-person, cmd-reset-all-community, cmd-reset-all-person, cmd-modlog-off, cmd-modlog-set,
modlog-flagged, modlog-label-score, upload-failed, violation, violation-action, modlog-violation, digest-head,
digest-summary, digest-none, digest-person, action-mute, action-unmute, action-disconnect, action-timeout,
action-done, action-skipped-off, action-skipped-observe, action-already-muted, action-not-connected, action-
audit-reason, action-not-allowed, action-failed, action-for, no-speak, value-on, value-off, spoken-ms,
spoken-s, spoken-min, spoken-h, spoken-d, dur-unlimited, dur-ms, dur-s, dur-min, dur-h, dur-d, presence-
paused, presence-nobody, presence-watching
```

</details>

### `settings.ftl` (136 keys)

<details>
<summary>Show keys</summary>

```
section-tracking, section-detection, section-warning, section-escalation, section-greeting, section-reporting,
section-recording, section-commands, section-system, source-builtin, source-file, source-global, source-
server, source-person, docs-scopes, docs-who, docs-default, scope-global, scope-server, scope-person, who-
admins, who-owner, apply-live, apply-reconnect, label-privacy_asking_for_pii, label-discriminatory, label-
harassment, label-sexual_content, label-illegal_and_regulated_content, label-dating_and_romantic_content,
label-profanity, label-disruptive_audio, setting-label-enabled, setting-label-threshold, setting-paused,
setting-guild-allowlist, setting-tracked-everywhere, setting-allow-e2ee-downgrade, setting-join-settle,
setting-leave-grace, setting-threshold, setting-strikes, setting-strike-window, setting-end-silence, setting-
max-sentence, setting-min-voiced, setting-max-reaction-delay, setting-observe-only, setting-audience, setting-
volume-db, setting-voice-language, setting-fallback-languages, setting-tts-voices, setting-speech-rate,
setting-no-speak-policy, setting-strike-notice, setting-announce-actions, setting-violation-window, setting-
escalation, setting-actions-enabled, setting-greet-enabled, setting-modlog-channel, setting-modlog-audio,
setting-owner-dm-audio, setting-digest, setting-digest-time, setting-digest-weekday, setting-timezone,
setting-jar-enabled, setting-chat-language, setting-recordings, setting-admins-play-audio, setting-commands-
enabled, setting-command-prefix, setting-admin-user-ids, setting-admin-role-ids, setting-instance, setting-ui-
url, setting-allowed-hosts, setting-cpu-threads, setting-tts-threads, choice-audience-offender, choice-
audience-tracked, choice-audience-channel, choice-no-speak-policy-text, choice-no-speak-policy-log, choice-
digest-off, choice-digest-daily, choice-digest-weekly, choice-weekday-monday, choice-weekday-tuesday, choice-
weekday-wednesday, choice-weekday-thursday, choice-weekday-friday, choice-weekday-saturday, choice-weekday-
sunday, choice-recordings-off, choice-recordings-flagged, choice-recordings-all, choice-step-action-none,
choice-step-action-mute, choice-step-action-disconnect, choice-step-action-timeout, choice-voice-language-
auto, err-setting, err-unknown-setting, err-scope, err-owner-only, err-not-probability, err-below-one, err-
not-whole, err-too-large, err-not-duration, err-negative, err-not-positive, err-below-frame, err-not-finite,
err-not-above-zero, err-not-time-of-day, err-unknown-tz, err-not-origin, err-origin-with-path, err-not-host,
err-prefix-spaces, err-not-lang, err-not-id, err-not-choice, err-no-steps, err-step-order, err-timeout-too-
long, err-step, err-not-number, err-not-switch, err-not-text, err-not-list, err-unknown-field
```

</details>

### `setup.ftl` (38 keys)

<details>
<summary>Show keys</summary>

```
setup-step-code, setup-step-instance, setup-step-token, setup-step-secret, setup-step-owner, setup-continue,
setup-done, setup-open, setup-code-help, setup-code-label, setup-code-wait, setup-code-wrong, setup-instance-
help, setup-instance-label, setup-token-help, setup-token-label, setup-token-format, setup-token-rejected,
setup-token-unreachable, setup-token-timeout, setup-secret-help, setup-secret-address, setup-secret-label,
setup-bot-ready, setup-owner-help, setup-owner-trouble, setup-secret-again, setup-expired, login-not-ready,
login-no-token, login-no-secret, login-unreachable, login-failed, login-stale, login-no-access, login-again,
form-expired, login-redirect-not-registered
```

</details>

### `ui.ftl` (332 keys)

<details>
<summary>Show keys</summary>

```
ui-ago-now, ui-ago-s, ui-ago-min, ui-ago-h, ui-ago-d, ui-station-recording, ui-station-cut, ui-station-queued,
ui-station-model, ui-station-verdict, ui-station-decision, ui-decision-clear, ui-decision-invalid, ui-
decision-untracked, ui-decision-strike, ui-decision-warn, ui-decision-observe, ui-decision-late, ui-nav-live,
ui-nav-voice-lines, ui-nav-reports, ui-nav-audit, ui-nav-system, ui-nav-communities, ui-nav-logout, ui-
offline, ui-auth-expired, ui-log-in-again, ui-wall-title, ui-wall-empty, ui-wall-violations, ui-wall-no-
violations, ui-lag, ui-not-in-voice, ui-in-channel, ui-log-in, ui-on, ui-off, ui-none, ui-save, ui-use-
inherited, ui-set-here, ui-inherited, ui-owner-only, ui-duration, ui-duration-or-unlimited, ui-voice-default,
ui-esc-from, ui-esc-action, ui-esc-duration, ui-esc-owner, ui-esc-help, ui-saved, ui-unchanged, ui-not-
allowed, ui-not-found, ui-tab-overview, ui-tab-settings, ui-tab-live, ui-tab-history, ui-tab-evidence, ui-
paused, ui-unavailable, ui-everywhere, ui-not-tracked, ui-tracked, ui-muted, ui-deafened, ui-calls, ui-no-
calls, ui-bot-listens, ui-bot-cannot-speak, ui-encrypted, ui-tracked-people, ui-nobody-tracked, ui-track, ui-
untrack, ui-track-someone, ui-user-id-or-mention, ui-track-help, ui-violations, ui-when, ui-who, ui-decision,
ui-older, ui-jar, ui-jar-empty, ui-jar-reset, ui-digest, ui-digest-help, ui-digest-send, ui-now, ui-today, ui-
in-window, ui-in-window-value, ui-next-step, ui-observe-only, ui-said-and-done, ui-nothing-yet, ui-conveyor,
ui-no-sentences, ui-no-evidence, ui-failed, ui-dropped-short, ui-dropped-echo, ui-play-warning, ui-play-
strike, ui-play-action, ui-play-greeting, ui-play-say, ui-say-now, ui-say-placeholder, ui-their-language, ui-
say, ui-say-help, ui-days, ui-day, ui-sentences, ui-flagged, ui-speech, ui-length, ui-scores, ui-delete-
recording, ui-built-in, ui-preview, ui-remove, ui-add-text, ui-add-clip, ui-clip-removed, ui-vl-help, ui-vl-
warning, ui-vl-any-type, ui-vl-any-step, ui-vl-step, ui-vl-greeting, ui-vl-strike, ui-vl-action-any, ui-vl-
action, ui-vl-say, ui-vl-say-new, ui-vl-name, ui-vl-text-placeholder, ui-vl-step-placeholder, ui-vl-preset-
placeholder, ui-vl-add-line, ui-clips, ui-clip-name, ui-clip-no-speech, ui-clip-transcript, ui-clip-heard, ui-
clip-sounds-like, ui-no-clips, ui-upload, ui-record, ui-record-stop, ui-record-uploading, ui-record-hint, ui-
all-kinds, ui-filter, ui-all-communities, ui-recording-kept, ui-heard-language, audit-by-bot, audit-by-file,
audit-by-old-bot, audit-globally, audit-in, audit-for, audit-set, audit-clear, audit-track, audit-untrack,
audit-voice-line, audit-imported, audit-action, audit-jar-reset, audit-jar-baseline, audit-recording-deleted,
audit-clip-saved, audit-clip-removed, audit-login, audit-msg-modlog, audit-msg-dm, audit-msg-digest, audit-
message-sent, audit-message-failed, audit-import, audit-log-repaired, audit-started, audit-stopped, audit-
stopped-unclean, audit-unknown, audit-group-settings, audit-group-actions, audit-group-jar, audit-group-
library, audit-group-logins, audit-group-messages, audit-group-bot, ui-status, ui-version, ui-microphones, ui-
models, ui-ready, ui-queues, ui-waiting, ui-done, ui-oldest, ui-parts, ui-restarts, ui-part-running, ui-part-
restarting, ui-part-not-answering, ui-part-stopped, ui-part-failed, ui-part-moderation, ui-part-undo, ui-part-
digest, ui-part-threads, ui-part-views, ui-part-gateway, ui-part-system, ui-storage, ui-log, ui-files, ui-
free, ui-index-behind, ui-fluxer-no-token, ui-fluxer-connecting, ui-fluxer-ready, ui-fluxer-reconnecting, ui-
fluxer-no-voice, ui-fluxer-rejected, ui-fluxer-secrets, ui-secrets-help, ui-client-secret-help, ui-replace,
ui-reconnect, ui-reload-settings, ui-clip-added, ui-clip-removed-notice, ui-clip-unreadable, ui-digest-not-
sent, ui-digest-sent, ui-jar-emptied, ui-no-longer-tracking, ui-no-such-clip, ui-not-a-line, ui-not-a-user,
ui-now-tracking, ui-reconnected, ui-recording-deleted, ui-reloaded, ui-said, ui-say-empty, ui-token-replaced,
ui-upload-empty, ui-upload-failed, ui-secret-from-env, perm-view-channel, perm-send-messages, perm-attach-
files, perm-add-reactions, perm-read-message-history, perm-connect, perm-speak, perm-mute-members, perm-move-
members, perm-moderate-members, perm-manage-guild, perm-administrator, ui-permissions, ui-perm-ok, ui-perm-
missing, ui-perm-modlog, ui-perm-actions, ui-invite, ui-invite-help, ui-clip-not-yours, ui-clip-added-by, ui-
track-the-bot, ui-tracked-everywhere, ui-digest-last-sent, ui-digest-failed, ui-logout-everywhere, ui-logout-
everywhere-help, ui-permissions-help, ui-say-own-text, ui-voices, ui-voices-help, ui-record-needs-https, ui-
record-refused, ui-record-upload-failed, ui-pause-here, ui-resume-here, err-not-connected, err-not-in-call,
err-bot-not-in-call, err-no-language, err-said-too-late, err-said-not-spoken, err-said-nothing, err-said-
failed, err-no-such-sentence, err-no-recording, err-log-halted, err-render, err-render-no-voice, err-render-
clip-missing, err-fluxer, err-bad-instance, err-store, err-unknown-host, err-bad-ui-address, err-no-login-
code, ui-fluxer-stopped, ui-model-classifier, ui-model-voice-activity, ui-model-speech, ui-model-not-
answering, ui-model-no-voices, ui-queue-scoring, ui-queue-speech, ui-log-halted, ui-log-retry, ui-log-writing-
again, ui-index-problem, ui-index-skipped, ui-bot-joining, ui-bot-retrying, ui-bot-leaving, ui-source
```

</details>

## Appendix D: The Settings In Alphabetical Order Of Their Keys

| # | Key |
| -: | --- |
| 1 | `setting-actions-enabled` |
| 2 | `setting-admin-role-ids` |
| 3 | `setting-admin-user-ids` |
| 4 | `setting-admins-play-audio` |
| 5 | `setting-allow-e2ee-downgrade` |
| 6 | `setting-allowed-hosts` |
| 7 | `setting-announce-actions` |
| 8 | `setting-audience` |
| 9 | `setting-chat-language` |
| 10 | `setting-command-prefix` |
| 11 | `setting-commands-enabled` |
| 12 | `setting-cpu-threads` |
| 13 | `setting-digest` |
| 14 | `setting-digest-time` |
| 15 | `setting-digest-weekday` |
| 16 | `setting-end-silence` |
| 17 | `setting-escalation` |
| 18 | `setting-fallback-languages` |
| 19 | `setting-greet-enabled` |
| 20 | `setting-guild-allowlist` |
| 21 | `setting-instance` |
| 22 | `setting-jar-enabled` |
| 23 | `setting-join-settle` |
| 24 | `setting-label-enabled` |
| 25 | `setting-label-threshold` |
| 26 | `setting-leave-grace` |
| 27 | `setting-max-reaction-delay` |
| 28 | `setting-max-sentence` |
| 29 | `setting-min-voiced` |
| 30 | `setting-modlog-audio` |
| 31 | `setting-modlog-channel` |
| 32 | `setting-no-speak-policy` |
| 33 | `setting-observe-only` |
| 34 | `setting-owner-dm-audio` |
| 35 | `setting-paused` |
| 36 | `setting-recordings` |
| 37 | `setting-speech-rate` |
| 38 | `setting-strike-notice` |
| 39 | `setting-strike-window` |
| 40 | `setting-strikes` |
| 41 | `setting-threshold` |
| 42 | `setting-timezone` |
| 43 | `setting-tracked-everywhere` |
| 44 | `setting-tts-threads` |
| 45 | `setting-tts-voices` |
| 46 | `setting-ui-url` |
| 47 | `setting-violation-window` |
| 48 | `setting-voice-language` |
| 49 | `setting-volume-db` |

## Appendix E: Numbers That Appear In The Docs

| Number | Appears as |
| -----: | ---------- |
| 0.6 | default threshold |
| 8790 | web port |
| 3 | exit code: another bot uses the volume |
| 78 | exit code: permanent problem |
| 12 | hours an owner stays logged in |
| 7 | days an admin stays logged in |
| 15 | minutes that count as a recent login for secret changes |
| 365.25 | maximum time-out length in days |
| 1.7 | GB of models and voices |
| 3 | GB of RAM at peak |
| 8 | GB of RAM to build |
| 25 | GB of disk to build |
| 30–60 | minutes for the first build |
| 4.4 | minimum Podman version |
| 5.2 | Podman version for the quadlet units |
| 21 | minimum clang version |
| 10001 | uid of the user inside the container |
| 0600 | mode of `secrets.toml` |

## Licences

The bot: AGPL-3.0-or-later (`LICENSE`); if you run a changed version for others, offer them its source (the web
page links to it). The Roblox voice-safety classifier: Roblox's model licence (next to the weights); Silero VAD: MIT;
Piper voices: see their model cards (Thorsten-Voice: CC0; lessac: the Blizzard 2013 licence); espeak-ng:
GPL-3.0-or-later; LiveKit's libwebrtc: BSD-3-Clause.

---

*This README is mostly filler. The filler is, at least, about this repository. The wombat has been dismissed.*
