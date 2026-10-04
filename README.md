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

> 🍄 **TL;DR, and please sit down first:** it listens to your friends swear and tells them off with a recorded voice. 🎙️🤬➡️📢
> It is written in Rust 🦀 because C is a loaded gun with a pointer on the trigger, and Windows is the same gun with an update
> dialog on top. It runs in rootless Podman 🐳 because root is for people who enjoy incident reports. There is an *unreasonable*
> amount of README below. 📜 The useful part is at the top. The rest is what happens when a developer stares at `cargo build`
> for 60 minutes and the compiler starts staring back. 👁️

- [Requirements](#requirements)
- [1. Create the bot in Fluxer](#1-create-the-bot-in-fluxer)
- [2. Run it](#2-run-it): [Podman](#with-podman), [systemd / Fedora CoreOS](#with-systemd-quadlets-fedora-coreos),
  [without a container](#without-a-container)
- [3. Set it up in the web page](#3-set-it-up-in-the-web-page)
- [Using it](#using-it) · [Settings](#settings) · [HTTPS](#https) · [Everyday commands](#everyday-commands)
- [Moving from the Python bot](#moving-from-the-python-bot) · [Troubleshooting](#troubleshooting) ·
  [Privacy](#privacy-and-data) · [Development](#development) · [Licences](#licences)

**Everything else (mostly unnecessary, increasingly unhinged):**

- [Prologue](#prologue)
- [Quick Facts Nobody Asked For](#quick-facts-nobody-asked-for)
- [Repository Statistics Dashboard](#repository-statistics-dashboard)
- [The Layer Cake](#the-layer-cake)
- [The Crate Gallery](#the-crate-gallery)
- [Biggest Files In The Repository](#biggest-files-in-the-repository)
- [The Five Warning Clips](#the-five-warning-clips)
- [The Eight Labels](#the-eight-labels)
- [Every Setting, With Commentary](#every-setting-with-commentary)
- [The Threshold Scale](#the-threshold-scale)
- [The Ladder Of Consequences](#the-ladder-of-consequences)
- [Chat Commands, Reviewed](#chat-commands-reviewed)
- [The CLI, Reviewed](#the-cli-reviewed)
- [Exit Codes](#exit-codes)
- [Tech Stack Trivia](#tech-stack-trivia)
- [Dependencies I Have Feelings About](#dependencies-i-have-feelings-about)
- [Cargo.lock Trivia](#cargolock-trivia)
- [Git History Trivia](#git-history-trivia)
- [Docs Folder Trivia](#docs-folder-trivia)
- [Container Lore](#container-lore)
- [Windows, A Eulogy](#windows-a-eulogy)
- [C, The Loaded Gun](#c-the-loaded-gun)
- [Segfault Support Group](#segfault-support-group)
- [The Unix Philosophy Tribunal](#the-unix-philosophy-tribunal)
- [The Wisdom Of Stack Overflow](#the-wisdom-of-stack-overflow)
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
- [Rants, Hot Takes And Holy Wars](#rants-hot-takes-and-holy-wars)
- [More Real Numbers](#more-real-numbers)
- [Mailing List Mode](#mailing-list-mode)
- [Insider Gags Glossary](#insider-gags-glossary)
- [Developer Bingo](#developer-bingo)
- [The Man Page](#the-man-page)
- [Compiler Error Therapy](#compiler-error-therapy)
- [Commit Message Hall Of Fame](#commit-message-hall-of-fame)
- [Appendix A to C: every Rust file, every package, every translation key](#appendix-a-every-rust-file-in-this-repository)
- [Appendix D: The Settings In Alphabetical Order Of Their Keys](#appendix-d-the-settings-in-alphabetical-order-of-their-keys)
- [Appendix E: Numbers That Appear In The Docs](#appendix-e-numbers-that-appear-in-the-docs)

## Requirements

> 🧠 **Repo fact:** the build wants 8 GB of RAM and 25 GB of disk because LiveKit's libwebrtc is built against Chromium's libc++. Your 32 crates barely register. Chromium is out there somewhere, laughing, in 400 MB chunks. 🧊
> Requirement #0, unlisted: Linux. Windows users may press `Alt+F4` now. 🪟🔫

- **System**: Linux. For the container: **Podman 4.4+** (rootless is fine; 5.2+ for the systemd units in `deploy/quadlet/`)
- **CPU**: x86-64 with AVX2, AES and BMI2 (most CPUs since about 2014), or ARMv8 with the crypto extensions
- **Memory**: about **3 GB** at peak while running; building needs about 8 GB and 25 GB of disk
- **Network**: outgoing internet including **UDP** (voice); **TCP 8790** reachable in your network for the web page
- **Fluxer**: a bot application, and someone with **Manage community** or **Administrator** to invite it

Run **one** bot per bot token.

## 1. Create the bot in Fluxer

> 🐙 **Fluxer fact:** the bot token looks like `<application id>.<secret>`. In the repo it is wrapped in `secrecy`, so it never shows up in logs. It wears a ski mask. 🥷 Never paste it into a chat. Not even a nice chat. Not even *this* chat.

1. In Fluxer open **User Settings → Applications**, create an application and copy its **Bot token**
   (`<application id>.<secret>`) and its **Client secret**. Keep both private; if they leak, reset them there.
2. Tell the people you will track that the bot listens to them (see *Privacy and data*).

The bot is invited to your community after the setup (step 3), with a link from its web page.

## 2. Run it

> 🦀 **Run fact:** the first start prints a setup code. It is not in the repo, not in the Containerfile and not in your heart. It is in the log. `RTFL`: read the fucking log. 📖

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

> 🍃 **Web fact:** the web UI is Rust compiled to WebAssembly. The `cargo xtask ci` has a *shipped-JavaScript gate*, so nobody can smuggle in a `left-pad` at night. The author fears `node_modules`. `node_modules` knows. 🕳️

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

> 🎙️ **Usage fact:** all 5 warning clips back to back are 13.09 seconds, which is shorter than a TikTok and has a better plot.

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

> 🎛️ **Settings fact:** 49 settings with an English label. `/etc` would be proud. `/etc` would also be confused by the web page. Reviews of each setting live in *Every Setting, With Commentary* below.

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

> 🔐 **HTTPS fact:** the bot's *outgoing* TLS uses the pure-Rust `graviola` crypto provider. OpenSSL is not invited. We do not speak of OpenSSL. Heartbleed sends its regards. 💔

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

> 🧰 **Command fact:** `pb doctor` is the best subcommand name. It does not prescribe antibiotics. It prescribes `read the log`.

With Podman (`podman exec profanity-watch pb …`), or `pb …` directly without a container:

- **Log**: `podman logs -f profanity-watch` (also daily files in `<data>/logs`)
- **Health**: `pb health`
- **Check the installation**: `pb doctor`
- **Setup code**: `pb setup-code`
- **Lost access / redo the setup**: `pb reset-setup`, then restart (token, secret, settings and data stay)
- **Check the event log**: `pb store verify`
- **Rebuild the search index**: stop the bot, then `podman run --rm -v profanity-watch-data:/data:U profanity-watch store rebuild-index`
- **Back up**: `podman volume export profanity-watch-data -o pb-data.tar`

## Moving from the Python bot

> 🐍 **Migration fact:** the old bot was Python. The importer is the only crate whose whole job is saying goodbye. 🪦 `pip install --break-system-packages` was the last thing it ever heard.

Import the old data directory into a new, empty volume before the first start:

```bash
podman run --rm -v profanity-watch-data:/data:U -v proofanitybot-data:/old:ro profanity-watch import --from /old
```

Settings, tracked people, voice-line texts and clips, history and violations, recordings, the audit trail, swear-jar
counts, pending timed mutes, installed voices and the secrets are carried over; `import-report.txt` lists what was done
and which old settings no longer exist (the old caps).

## Troubleshooting

> 🛠️ **Troubleshooting fact:** exit code 78 is a lot of the problems. 78 is `EX_CONFIG` from the BSD `sysexits.h`. Somebody in the 1980s saw your pain coming. 🧙

- **The page does not open**: container running (`podman ps`)? same network? firewall (`8790/tcp`)? open it by IP
- **"This address is not one of the bot's web UI addresses"**: open it by IP, then set *System → Web UI address* or *Extra host names*
- **Login fails at Fluxer**: the redirect address is not registered exactly (setup step 4; the login names the address), or the client secret is wrong (enter it again on the setup's last step, or *System → Client secret*)
- **"Record a clip" is greyed out**: the page is not opened over HTTPS (see *HTTPS*) or on localhost; upload a file instead
- **Exit code 78**: the log says why (configuration, model files, CPU); `pb doctor` checks everything
- **Exit code 3**: another bot process uses the same data directory
- **`rustc: symbol lookup error: …librustc_driver….so: undefined symbol …`**: the distribution's Rust package does not match its LLVM libraries (a partial update, or packages from different repositories): install rustup instead (*Development*)
- **`rustup could not choose a version of cargo to run`**: `rustup default nightly`, then in the project directory `rustup toolchain install`
- **`can't find crate for core` … `wasm32-unknown-unknown`**: the browser target is missing: in the project directory `rustup toolchain install` (or `rustup target add wasm32-unknown-unknown`)
- **`target/release/pb`: unknown command / no such file**: the build before it failed: scroll up to its first error
- ***System* says the instance has voice turned off**: that Fluxer instance has no voice calls; nothing for the bot to do there
- **Does not join voice**: person not tracked or paused, missing Connect, or an end-to-end encrypted call (setting *Join end-to-end encrypted calls*)
- **Flagged but no warning**: strikes not reached yet, *Observe only (silent)* is on, the person is deafened, or the bot may not speak (it writes in the chat instead)

## Privacy and data

> 🔍 **Privacy fact:** the event log is append-only and hash-chained. Edit one entry by hand and `pb store verify` snitches. The hash chain is the repo's Stasi. ⛓️

Audio is processed in memory, on your machine. A sentence's recording is kept only when it was **flagged** (the owner
can switch *Recordings* to every sentence or to none); recordings stay until the owner deletes them (person page →
Recordings). Scores, decisions and every change are kept in an append-only, hash-chained event log in the data
directory; nothing is deleted automatically. Recordings go to the owner's direct messages and, if enabled, to the
mod-log channel. The bot is visible in the call while it listens.

**Recording or analysing people's voices may need their consent where you live: tell the people you track.**

## Development

> 💻 **Development fact:** `cargo xtask ci` runs fmt, clippy (native and wasm), tests, cargo-deny, cargo-shear, the layer rules, and the zero-C and shipped-JavaScript gates. Nobody gets in. Not even you. Especially not you. 🚪

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

## Prologue

Welcome. Hello. Are you sitting? Good. The floor is breathing a little, that is normal, it is the fan on your build machine.

This README is long. The *real* documentation is the numbered setup steps, the warnings about tokens, and the troubleshooting
list. Everything else is, to put it kindly, *colour*, and to put it unkindly, a cry for help with footnotes. 🆘

**How to read this README:**

1. Skim the intro. It is the actual project description.
2. Jump to **Requirements** if you want to run the bot.
3. Ignore the rest. You will not. Nobody does. The sunk-cost fallacy has already got you.

> ⚠️ **Content warning:** from here on there is swearing, aimed at *software*, never at people. 🤬 This is a profanity bot, so this
> README would absolutely get its own author warned. The irony has been noted, framed, and hung next to the diploma. 🖼️

> 🍄 **Trip report #1:** the terminal blinked. The cursor blinked back. We both knew `cargo build` was at 3%. Nobody spoke.

**Legend:**

- 🧠 a fact about this repository that nobody asked for
- 🦀 Rust-related nonsense
- 🐙 Fluxer-related nonsense
- 🎙️ voice-related nonsense
- 📢 a warning clip is mentioned
- 🚫 do not do this
- 🐧 a Linux-flavoured rant
- 🪟 a Windows-flavoured funeral
- ☢️ C, undefined behaviour, or both
- 🤬 the author yells at software, affectionately
- 💩 a strong opinion about tooling
- 🔥 a hot take
- 💀 pain, mostly from the build
- 🪦 the Python bot, rest in peace
- 🍝 spaghetti (there is none, see the layer cake)
- 🍄 trip report, sanity not included

## Quick Facts Nobody Asked For

All numbers below are real. We counted. We should not have, but we did, and now we cannot stop. Send help. Or coffee. Or `kill -9`.

- 🤷 **32 crates** in `crates/`. That is a lot of crates for a bot that says "watch your mouth, buddy".
- 📏 About **49,581 lines** of Rust in **235 files**. Linux itself is bigger, but Linux also has Torvalds, so it has an excuse.
- 🐘 The biggest crate, `pb-engine`, has **8,388 lines**. The smallest, `pb-voice-api`, has **161**. It is small, but it knows what it is doing. Unlike `systemd`.
- 🎭 The biggest single file is `crates/pb-fluxer-fake/src/lib.rs`, **1,270 lines**: a fake Fluxer so the real one is not bothered during tests. A one-file theatre. No refunds.
- 🔒 `Cargo.lock` lists **1,079 packages**. The bot has 32 of its own. The other 1,047 are friends we met on the way, and we did not do a background check. 🧾
- 🧪 **173 tests**, **23** of them `#[ignore]` because they need models, a LiveKit server or a browser. They are the introverts of the test suite.
- 📝 The word `TODO` appears **0 times**. Clippy's `todo` lint is on `warn`. Either the author is disciplined or lying. Both are valid.
- 😬 `unwrap()` appears **467 times**, mostly in tests. "Mostly" is doing the heavy lifting of a Hetzner root server.
- 🐑 `clone()` appears **859 times**. Rust developers call this "being pragmatic". C developers call it "cheating". Python developers call it "Tuesday".
- ⏳ `async fn` appears **483 times**. The bot's main job is waiting. Like an admin on a Friday at 16:55.
- 🏗️ **435** `struct` mentions, **144** `enum` mentions, **21** `trait` mentions. Enums are the minority. Enums feel left out. Enums have started a union.
- 📚 **2,091** lines of `///` doc comments. The author really, *really* wanted you to understand. They did not trust you, but they did want you to understand.
- ☢️ The workspace sets `unsafe_code` to **deny**. The only `unsafe` lives in `pb-espeak`, the crate that talks to C. It is the designated smoking area. 🚬 Scheiß C.
- ⚖️ Default threshold: **0.6**. The bot is 60% judgemental by default. Like a Debian mailing list.
- 🔌 Default web port **8790**. Not prime. Neither is the bot's moral standing.
- 🐳 The `Containerfile` has **3 stages** and runs as user `10001`, not root. Root is for incident reports. We covered this.
- 📢 **5 warning clips**, **13.09 seconds** total. That is less time than `apt update` takes to say "Err:3".
- 📅 **25 commits** on the main line by 3 authors, counting the AI. Every one of them was made on a **Sunday**. This is a Sunday project. The weekend was the product. Touching grass was cancelled.
- 🔧 The toolchain file asks for **nightly** (with wasm32), while `Cargo.toml` says `rust-version` **1.99**, edition **2024**. The code stays within stable Rust, so nightly is a lifestyle choice, like running Gentoo.
- 🧠 About **3 GB** of RAM at peak, about **1.7 GB** of model weights. In the Electron universe that is "a small desktop app that does nothing".
- 🌍 **8 Fluent files**, **1,670 lines**, two languages. Both languages say the same thing, with different punctuation and the same disappointment.

> 🍄 **Trip report #2:** a crab rode a penguin through a field of semicolons. Neither had a semicolon. The crab did not need one.

## Repository Statistics Dashboard

*Formerly a table. Tables are now banned in this README. A dashboard of bullets is still a dashboard. Fight me in the issues.* 📉

- **Crates**: 32
- **Rust files in `crates/`**: 235
- **Lines of Rust in `crates/`**: 49,581
- **Average lines per crate**: 1,549
- **Average lines per file**: 210
- **Packages in `Cargo.lock`**: 1,079
- **Direct third-party workspace dependencies**: 52
- **Tests**: 173
- **Ignored tests**: 23
- **`pub fn`**: 694
- **`async fn`**: 483
- **`Arc<` mentions**: 246
- **`Mutex` mentions**: 129
- **`format!` calls**: 406
- **`println!` calls**: 81
- **`///` doc lines**: 2,091
- **Settings with an English label**: 49
- **Translation keys (English)**: 601
- **Fluent lines (de + en)**: 1,670
- **Warning clips**: 5
- **Total warning audio**: 13.09 s
- **Size of warning audio on disk**: 1228 KiB
- **Model weights to download**: ~1.7 GB
- **RAM at peak**: ~3 GB
- **Build time**: 30–60 min
- **Build disk**: ~25 GB
- **Commits (main line)**: 25
- **Words in the real README**: 3,020
- **Words in this README**: a lot more

## The Layer Cake

The crates are sorted into layers, and the rules for who may depend on whom are enforced by `cargo xtask deps` (see `xtask/layers.toml`).
Imagine a cake. Every layer is a flavour of responsibility, and the cake may only be eaten from the top. 🎂 Lower layers are not allowed to
look at higher layers. It is basically a hierarchy with a linter.

- **L0**: the pure ones. No I/O. They have never seen a network packet and are happy. Monks. Haskellers. 🧘
- **L1**: the interfaces. Contract-minded. Lawyers.
- **L2**: the doers. Models, Fluxer, voice, storage. The sponge. Where the suffering is.
- **L3**: `pb-live` and `pb-engine`. The brain and the gossip hub.
- **L4**: `pb-web` and `pb-web-server`. The face. The frosting.
- **L5**: the binary, the testkit, the fake Fluxer, the devstack, `xtask`. The sprinkles. Also the thing that actually runs. The sprinkles are the cake. Whatever.

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

The third-party crates are confined too. `livekit` may only be used by `pb-voice-livekit` and `pb-testkit`; `burn` only by
`pb-classifier-roblox` (the VAD got kicked out of the Burn club); `rten` only by `pb-tts-piper`; `turso` only by `pb-store` and `pb-import`.
This is the software equivalent of a seating chart at a wedding, and `cargo xtask deps` is the aunt who enforces it. 💒

## The Crate Gallery

Every crate in `crates/`, with its real line count and an opinion nobody requested. In the order the filesystem coughed them up. 🫗

- `pb` (L5, 2,530 lines): The binary. The only crate that gets to have `main`. The other 31 are a very elaborate fan club. 🍒
- `pb-audio` (L2, 541 lines): Decode, resample, loudness, fades. Sound goes in, slightly different sound comes out, and nobody is allowed to say `ffmpeg`.
- `pb-classifier-roblox` (L2, 1,143 lines): The judge. Runs Roblox's voice-safety model in Burn. Roblox spent years learning what children scream in lobbies, and we inherit the trauma for free. 🧑‍⚖️
- `pb-commands` (L0, 435 lines): Parses `!pb`. IRC bots did this in 1998 and nobody has improved on it since.
- `pb-devstack` (L5, 268 lines): A fake Fluxer with fake friends in a fake call. Never shipped. Best social life in the repo. 🎭
- `pb-domain` (L0, 774 lines): Ids, labels, scores, verdicts. The nouns. No I/O, no regrets, no side effects, like a monk or a Haskell programmer.
- `pb-engine` (L3, 8,388 lines): Follows people into voice and decides what to do about them. Also where `git blame` points when anything breaks. 🧠
- `pb-espeak` (L2, 294 lines): Four C functions in a trench coat. The `unsafe` lives here and is not allowed out. Scheiß C, but we need the phonemes. 🚬
- `pb-fluxer` (L2, 2,584 lines): Talks to Fluxer. Respects rate limits, unlike every Discord bot you have ever met.
- `pb-fluxer-api` (L1, 774 lines): The shape of Fluxer, so nobody else has to look at it directly.
- `pb-fluxer-fake` (L5, 1,270 lines): One file. Pretends to be an entire chat platform. Stage fright: none. 🎭
- `pb-i18n` (L0, 739 lines): Two languages, de and en, so the bot can tell you off in the language you swore in.
- `pb-import` (L2, 1,511 lines): Eats the old Python bot's data. Silently. With respect for the dead. 🪦
- `pb-infer` (L2, 1,264 lines): Model threads and priority queues. Your sentences wait in line like at the Bürgeramt. 🎟️
- `pb-live` (L3, 955 lines): The gossip hub. Pushes updates to your browser before you finish blinking.
- `pb-live-proto` (L0, 1,876 lines): The grammar of the gossip. Compiles to wasm so the browser can gossip too. No JavaScript was harmed. None was invited.
- `pb-models-api` (L1, 304 lines): Interfaces for VAD, classifier and TTS, with contract tests. A prenup for models.
- `pb-policy` (L0, 1,695 lines): Strikes, escalation, the follow machine. Judge Dredd with a config file. ⚖️
- `pb-segment` (L0, 912 lines): Cuts speech into sentences. A very small butcher with hysteresis.
- `pb-settings` (L0, 2,997 lines): Almost 3,000 lines to say "it depends". Enterprise grade.
- `pb-store` (L2, 2,864 lines): The hash-chained diary. Trusts nobody, including itself. 📓
- `pb-store-api` (L1, 1,815 lines): The front cover of the diary.
- `pb-testkit` (L5, 1,116 lines): The cast of the test suite: fake participants, fake LiveKit, golden data. Method acting. 🎬
- `pb-tls` (L2, 218 lines): Pure-Rust TLS setup and a CPU check. OpenSSL was not invited. It is still not over it. 💔
- `pb-tts-piper` (L2, 836 lines): Makes the bot talk. The bot did not ask for a voice. Neither did you.
- `pb-vad-silero` (L2, 790 lines): Human or fridge? Ten times faster since we replaced the framework with a `for` loop. 🧊
- `pb-voice-api` (L1, 161 lines): The smallest crate. Hello-world energy, peak performance.
- `pb-voice-livekit` (L2, 765 lines): Where audio enters and leaves. The only crate that has seen UDP in the wild. 📡
- `pb-voicelines` (L0, 1,090 lines): Slots, templates, utterance plans. The bot's writers' room.
- `pb-web` (L4, 4,163 lines): Leptos. Rust in the browser, on purpose, on a Sunday. 🍃
- `pb-web-server` (L4, 4,124 lines): axum. The bouncer: host allowlist, sessions, uploads. "Not on the list, not coming in." 🚪
- `pb-weights` (L2, 385 lines): Downloads 1.7 GB, checks the SHA-256 like customs, resumes if interrupted. `curl | sh` could never. 🛃

**Size ranking, in one breath:** `pb-engine` (8,388) is bigger than `pb-web` (4,163) and `pb-web-server` (4,124), which beat `pb-settings` (2,997), `pb-store` (2,864), `pb-fluxer` (2,584) and `pb` (2,530). Everyone else is a rounding error, and `pb-voice-api` (161) is the smallest rounding error of them all.

## Biggest Files In The Repository

The ten thickest `.rs` files. They have eaten well. None of them is a god object. (Some of them are a *minor deity*.) 🛐

- `crates/pb-fluxer-fake/src/lib.rs`: 1,270 lines
- `crates/pb-settings/src/v1/values.rs`: 1,037 lines
- `crates/pb-web-server/tests/routes.rs`: 928 lines
- `crates/pb-engine/src/v1/engine.rs`: 904 lines
- `crates/pb-live-proto/src/v1/state.rs`: 881 lines
- `crates/pb-settings/src/v1/schema.rs`: 806 lines
- `crates/pb-fluxer/src/v1/gateway.rs`: 791 lines
- `crates/pb-vad-silero/src/lib.rs`: 709 lines
- `crates/pb-fluxer/src/v1/rest.rs`: 693 lines
- `crates/pb-policy/src/v1/follow.rs`: 675 lines

For reference, the `Cargo.lock` has 11,939 lines and nobody has ever read them. Nobody. Not even the lock.

## The Five Warning Clips

The repo ships exactly 5 warning clips in `clips/` (see `clips/clips.json`), each with weight 1.0. If nothing else is configured, the bot picks one at random whenever it tells somebody off. Randomness by `fastrand`, fate by Podman.

- 📢 **"Hey! Watch your language."** (3.36 s, 315 KiB): the classic. The longest, because it has an exclamation mark and a lot to say. ★★★★☆
- 📢 **"Language! Cut it out."** (3.00 s, 281 KiB): sounds like a PE teacher who has had enough of everyone, forever. ★★★★★
- 📢 **"Easy on the swearing, please."** (2.33 s, 219 KiB): the polite one. The "please" carries a whole microservice. ★★★☆☆
- 📢 **"Watch your mouth, buddy."** (2.26 s, 212 KiB): the "buddy" is passive-aggressive. We love it. ★★★★★
- 📢 **"Hey, keep it clean."** (2.14 s, 201 KiB): the shortest. The haiku of warnings. ★★★★☆

All five are 16-bit, 48,000 Hz. **Total: 13.09 seconds of disappointment, 1228 KiB on disk.** Less than one npm package that "just adds a left-pad".

**Fun with audio maths:**

- Probability that you get the "buddy" clip: 1 in 5, so 20%. Fate is cruel, like `rm` with a typo.
- A 2.33 s clip at 48 kHz has roughly 111,840 samples. Each one knows exactly what it is doing, which is more than can be said for your last deploy.
- If you upload your own clip, it is normalised and checked by the classifier. Yes, the classifier checks the warning for swearing. A warning that swears would be embarrassing. It would also be the most honest commit in the history.
- Reading the useful part of this README out loud takes about 20 minutes. Reading all of it takes an entire shift. Do not.

> 🍄 **Trip report #3:** the five clips met in a forest. "Buddy" spoke first. The forest apologised.

## The Eight Labels

The classifier scores speech against eight labels. Each one can be turned on or off and has its own threshold (`setting-label-enabled`, `setting-label-threshold`).

- **Asking for personal info**: Asking for personal info: 'what's your address' in a voice call. Please do not.
- **Discriminatory**: Discriminatory. Not funny, not allowed.
- **Harassment**: Harassment. The bot is not your friend today.
- **Sexual content**: Sexual content. We are not making a joke here. The classifier has heard things.
- **Illegal and regulated**: Illegal and regulated. The bot is not a lawyer, only a mildly judgemental listener.
- **Dating and romance**: Dating and romance. Yes, the bot can flag you flirting. Yes, this is awkward.
- **Profanity**: Profanity. The entire reason the repo exists. The star of the show.
- **Disruptive audio**: Disruptive audio. Screaming, air horns, and your cousin's karaoke.

## Every Setting, With Commentary

There are **49** settings with an English label. Real descriptions live in the web page and in `pb settings docs`. The commentary below is a service nobody ordered, like the `systemd-resolved` you did not ask for.

Reminder from the real docs: settings can be set globally, per community and per person; the most specific one wins (person > community > global > `config.toml` > built-in).

- **Warn about { $label }** (`setting-label-enabled`): One switch per label. Which kinds of bad do we care about today?
- **Threshold for { $label }** (`setting-label-threshold`): One bar per label. How bad is too bad.
- **Paused** (`setting-paused`): The bot takes a nap. It still has feelings. They are mostly `SIGSTOP`.
- **Only these communities** (`setting-guild-allowlist`): A guest list for communities. No invite, no entry. `iptables -P INPUT DROP`, but friendlier.
- **Tracked in every community** (`setting-tracked-everywhere`): Track someone in every community. Surveillance, but with a settings page.
- **Join end-to-end encrypted calls** (`setting-allow-e2ee-downgrade`): Join end-to-end encrypted calls anyway. Read the tooltip. Read it twice. Then stare at the ceiling for a while.
- **Join delay** (`setting-join-settle`): Wait a moment before joining voice. Politeness as a config value. `sleep 1`, but with feelings.
- **Leave delay** (`setting-leave-grace`): Wait a moment before leaving. The bot hates awkward exits. Like `:q!` in the wrong terminal.
- **General threshold** (`setting-threshold`): The general bar. Lower is stricter. Default 0.6. Like `sudoers`, except somebody can actually read it.
- **Strikes before a warning** (`setting-strikes`): How many offences before a warning. A baseball-adjacent number.
- **Strike window** (`setting-strike-window`): How long strikes count. Strikes expire like milk, but with a TTL.
- **Pause that ends a sentence** (`setting-end-silence`): How long a pause ends a sentence. The bot has strong opinions about commas. They are Oxford.
- **Longest sentence** (`setting-max-sentence`): The longest a sentence may be. Run-on sentences get a polite `SIGKILL`.
- **Shortest speech scored** (`setting-min-voiced`): The shortest speech worth scoring. "Hm" is not a sentence. "Hm" is a `NOP`.
- **Latest warning** (`setting-max-reaction-delay`): How late a warning may still arrive. Too late and it is just rude.
- **Observe only (silent)** (`setting-observe-only`): Silent mode. The bot judges you and tells nobody. `/dev/null` with opinions.
- **Who hears the warning** (`setting-audience`): Who hears the warning: the offender, the tracked people, or the channel. The public-shaming dial, from 0 to 11.
- **Warning volume** (`setting-volume-db`): Warning volume in dB. Not "yelling". Just "persuasive". Do not start a loudness war. EBU R128 will find you.
- **Spoken language** (`setting-voice-language`): Which language the warning speaks. `auto` guesses, and sometimes guesses German.
- **Fallback languages** (`setting-fallback-languages`): Plan B for languages. And plan C. And plan D.
- **Text-to-speech voices** (`setting-tts-voices`): Which text-to-speech voices are installed. A choir of robots.
- **Speech rate** (`setting-speech-rate`): How fast the robot talks. Slow means polite. Fast means 'I have places to be'.
- **Without the Speak permission** (`setting-no-speak-policy`): What to do without the Speak permission: write in chat, or log. The bot can only whisper.
- **Announce strikes** (`setting-strike-notice`): Announce strikes. 'That is one.'
- **Announce actions** (`setting-announce-actions`): Announce mutes and disconnects. Because the silence needs an explanation.
- **Count violations over** (`setting-violation-window`): Count violations over a period. Memory has a length.
- **Escalation steps** (`setting-escalation`): The ladder of consequences. See *The Ladder Of Consequences*. It has no `sudo`.
- **Allow moderation actions** (`setting-actions-enabled`): Allow moderation actions at all. The big red button. `rm -rf` for voice channels. Treat accordingly.
- **Greeting** (`setting-greet-enabled`): The bot says hello. It does this to be nice. It is not nice.
- **Mod log channel** (`setting-modlog-channel`): Where the bot snitches. Every flagged sentence gets a post. `/var/log/shame`.
- **Audio in the mod log** (`setting-modlog-audio`): Attach the audio in the mod log. Evidence, with sound.
- **Recordings in messages to the bot owner** (`setting-owner-dm-audio`): Send recordings to the owner in direct messages. Think of it as postcards.
- **Summary report** (`setting-digest`): Daily or weekly summary. A newsletter nobody subscribed to. A cron job with a hat.
- **Report time** (`setting-digest-time`): What time the summary arrives. Not 3 a.m. Probably.
- **Report day (weekly)** (`setting-digest-weekday`): Which day the weekly report arrives. Not Friday afternoon. Friday afternoon is a protected habitat.
- **Time zone** (`setting-timezone`): Time zone. The source of all bugs in all projects since 1884.
- **Swear jar** (`setting-jar-enabled`): The swear jar. A counter. No actual money. Calm down. No, it is not a coin, and no, it is not on a blockchain.
- **Chat language** (`setting-chat-language`): The language the bot writes in. `de` or `en`.
- **Recordings** (`setting-recordings`): Keep recordings for flagged, all, or no sentences. The privacy dial. The GDPR is watching and has no sense of humour.
- **Community admins may play recordings** (`setting-admins-play-audio`): Whether community admins may play recordings. Trust, but gated.
- **Chat commands** (`setting-commands-enabled`): Chat commands on or off. Silence the `!pb` crowd.
- **Command prefix** (`setting-command-prefix`): The prefix. Default `!pb`. Do not set it to a space. The validator will cry. So will you.
- **Extra bot owners** (`setting-admin-user-ids`): Extra bot owners. More hands, more risk.
- **Admin roles** (`setting-admin-role-ids`): Admin roles. Titles matter.
- **Fluxer instance** (`setting-instance`): Which Fluxer instance. `https://api.fluxer.app` or your own.
- **Web UI address** (`setting-ui-url`): The web UI address. Must match the redirect address. Matters.
- **Extra host names** (`setting-allowed-hosts`): Extra host names the page may be opened by. A guest list for URLs.
- **CPU threads for the model** (`setting-cpu-threads`): CPU threads for the model. More threads, more heat. Your fan has opinions and a union.
- **CPU threads for speech** (`setting-tts-threads`): CPU threads for speech. The robot voice needs cores too.

**The nine settings sections in the web page:** Tracking, Detection, Warning, Escalation, Greeting, Reporting, Recording, Chat commands and System. Read in that order it tells a story: *we track you, we detect you, we warn you, we escalate, we greet, we report, we record, we talk about it in chat, and then we fix the system.* That is also the plot of every office job.

## The Threshold Scale

Lower is stricter. The default is 0.6. The scale below is a feelings chart, because tables are dead and we killed them. 🔪

- **0.05** 🔥🔥🔥🔥🔥: flags a sneeze. A heavy sigh. Your stomach.
- **0.15** 🔥🔥🔥🔥🔥: flags "oh no". The bot arrests everyone who has opened `journalctl`.
- **0.30** 🔥🔥🔥🔥: flags "shoot" and "darn". Basically a church group.
- **0.45** 🔥🔥🔥: most mild language. Grandma mode.
- **0.60** 🔥🔥🔥: **the default.** The bot at peace. Strict but fair, like a Debian maintainer.
- **0.75** 🔥🔥: only the clear cases. Lets things slide, like a sysadmin after `rm -rf /tmp/*` "worked".
- **0.90** 🔥: basically a houseplant. A ficus with a classifier.
- **0.95** 🔥: needs a shouted, dramatic, three-part curse with a plot twist.
- **1.00** 🔥: never flags anything. Why run it. This is `/dev/null` with extra steps and a Containerfile.

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

The real step actions are `none`, `mute`, `disconnect` and `timeout`, and a step can also be marked *Tell the bot owner*. The ladder above is a simplification, but the vibe is correct, and vibes are 80% of all production incidents.

**Strikes needed, and how the bot feels about it:**

- **1**: strict. The bot has no sense of humour. Roughly a Rust compiler with `#![deny(warnings)]`.
- **2**: a fair warning. The bot's mood: neutral. Switzerland.
- **3**: baseball rules. Also the number of times you try before reading the docs.
- **4 to 5**: generous. The bot gives you the benefit of the doubt. The doubt has a weight limit.
- **6 to 7**: the bot is on holiday. It left a note: "back in Q3".
- **8 to 10**: the bot has left the building, taken the keys, and is now a `TODO` in somebody else's repo.

## Chat Commands, Reviewed

The bot reads commands in a text channel it can see, starting with `!pb` or a mention. A review of each:

- **`!pb help`**: Anyone — Lists everything. Like a menu with no prices.
- **`!pb status`**: Anyone — Tells you what the bot is up to. Most of the time: listening.
- **`!pb list`**: Anyone — Lists the tracked people. A hall of fame.
- **`!pb jar [@user]`**: Anyone — The swear jar counter. No coins. Only shame.
- **`!pb add @a`**: Admins — Follow these people into voice. "You're on the list now."
- **`!pb remove @a`**: Admins — Stop following them. A gentle breakup.
- **`!pb pause` / `resume`**: Admins — Nap time. Wake up.
- **`!pb observe on|off`**: Admins — Silent mode on or off.
- **`!pb set threshold 0.6 [@a]`**: Admins — Change how strict the bot is.
- **`!pb set strikes 2`**: Admins — Change the strikes.
- **`!pb set window 20s`**: Admins — Change the strike window.
- **`!pb set audience offender|tracked|channel`**: Admins — Choose who hears the warning.
- **`!pb set language de`**: Admins — Choose the language. `de` or `en`.
- **`!pb reset <setting|all> [@a]`**: Admins — Turn it off and on again, but for settings.
- **`!pb modlog #channel|off`**: Admins — Choose where the bot snitches.

German words work too: `an`/`aus` and `ja`/`nein`. The bot is bilingual, and a little pushy in both languages.

## The CLI, Reviewed

The `pb` binary has a few subcommands. A short summary of each, with feelings:

- **`pb run`**: Runs the bot. The default (`CMD ["run"]`). — 💪 Ready.
- **`pb doctor`**: Checks the installation. — 🧐 Concerned, but professional.
- **`pb health`**: Health check, used by the container's health check. — 💚 Alive.
- **`pb setup-code`**: Prints the first-start setup code. — 🤫 Secretive.
- **`pb reset-setup`**: Redo the setup. Token, secret, settings and data stay. — 🌱 Fresh start.
- **`pb fetch-weights`**: Downloads the models and voices (about 1.7 GB), resumable. — 🧘 Patient.
- **`pb import`**: Imports the old Python bot's data. — 🕯️ Respectful.
- **`pb store`**: Verifies the event log and rebuilds the search index. — 🕵️ Suspicious.
- **`pb settings`**: Prints all settings docs. — 🗣️ Chatty.

## Exit Codes

- **0**: Clean exit — 😌 Relieved
- **1**: A generic failure — 😢 Sad
- **2**: Bad command line usage — 😒 Annoyed
- **3**: **Another bot is using the same data directory** — 🐺 Territorial
- **78**: **A permanent problem: configuration, missing model files, a CPU without the needed instructions** — 🧙 Tired but wise
- **137**: Killed, probably for using too much memory — 💀 Grim
- **139**: Segmentation fault. In Rust. That would be rare. — 😱 Shocked
- **143**: Stopped with SIGTERM by Podman. Polite. — 🎩 Dignified

Fun trivia: the number 78 is the conventional "configuration error" exit code from the old BSD `sysexits.h` (`EX_CONFIG`). Back in the day someone sat down and picked 78. It is the repo's favourite number.

The container is not restarted on 78 or 3 until the problem is fixed. That is the rule. The bot has boundaries.

Exit code `139` in Rust would be rare. In C it is a Tuesday. In Windows it is a blue screen with a QR code that leads nowhere. 🪟💀

## Tech Stack Trivia

Quick, unrequested notes on the things this bot is built from. Every item is a rabbit hole. Do not follow the rabbit. The rabbit has `libc++`.

- 🦀 **Rust.** The language of the bot, the web page (via WebAssembly) and the build tool (`xtask`). Mascot: Ferris the crab. The crab does not talk in voice calls, but it has strong opinions about your lifetimes.
- 🎙️ **Silero VAD.** Voice activity detection, version 6.2. It used to run on Burn, but `docs/dependencies.md` says Burn spent most of each 0.2 ms step dispatching tiny operations, and the hand-written loops need about 0.02 ms. A tenfold speed-up by writing a `for` loop. "Just write the loop" is the oldest trick in the book. 🔁
- 🧠 **Roblox voice-safety classifier.** v3, in Burn. Roblox has decades of experience with children yelling in lobbies. We borrow the trauma.
- 🗣️ **Piper.** Neural text-to-speech, here on `rten` with `espeak-ng` as phonemizer. Thorsten-Voice for German, lessac for English. The bot has two voices and zero feelings.
- 📡 **LiveKit.** WebRTC for the calls, official Rust SDK. Its libwebrtc is built against Chromium's libc++, which is why the build takes an hour and the build machine feels so tired. Google compiled its own C++ standard library into a video-call stack, and now *you* wait. Thanks. Thanks a lot. 🙃
- 🗄️ **Turso.** A SQLite-compatible database written in Rust, vendored under `third_party/`. SQLite, but it took a gap year to rewrite itself.
- 🔥 **Burn.** A deep-learning framework in Rust. Vendored as `third_party/burn-flex`. Runs the classifier only, since the VAD left. Burn got dumped for a `for` loop. 💔
- 🌿 **`branches`.** A small vendored crate in `third_party/branches`, patched because nightly renamed `core::intrinsics::abort` and turso's dependency could not follow. This repo now contains a crate called `branches` *and* actual git branches. Related? No. Funny? Yes.
- 🌐 **axum.** The web server. Listens on 8790 like a shy Apache.
- 🍃 **Leptos.** The web UI. Server-rendered pages with interactive "islands". Islands: the only vacation this project has had.
- 📖 **Fluent.** Mozilla's localisation system for all texts, in `de` and `en`.
- 🐳 **Podman.** The recommended way to run the bot, with `--read-only --cap-drop=ALL --security-opt no-new-privileges`. The most polite container ever. A butler with a seccomp profile.
- 🧾 **systemd quadlets.** `deploy/quadlet/` has three units. systemd will eventually read this README to you, fix the typos, and become PID 1 of your thoughts. 🧵
- 🔐 **graviola.** A pure-Rust crypto provider for TLS. It checks your CPU like a bouncer checks IDs.
- 🐍 **Python.** The previous bot. Gone but not forgotten. The importer keeps its data alive. `IndentationError`, we hardly knew ye.

**Where does the code *not* look like Rust?** `docs/exceptions.toml` lists the few non-Rust pieces: LiveKit's libwebrtc and espeak-ng. The rest of the bot is pure, glorious, borrow-checked Rust. The exception list is 103 lines. Every other line is proud.

## Dependencies I Have Feelings About

The old dependency roster was a table with 52 rows. It has been taken out back. These are the ones that survived, and why.

- `tokio`: the async runtime. The heartbeat. Zero hearts, one runtime. Everything `.await`s on it, like a Bundestag committee.
- `serde`: serialises everything. If it is a struct, it is JSON by lunchtime.
- `anyhow` and `thiserror`: one makes errors easy, the other makes them *typed*. Rust devs pick whichever gives them less shame this week.
- `secrecy`: wraps secrets so they do not show up in logs. Your bot token wears a ski mask. 🥷
- `sha2`: the customs officer for 1.7 GB of weights.
- `toml_edit`: edits TOML and keeps your comments. A rare act of kindness in an industry that deletes comments on sight. 💕
- `clap`: parses `pb doctor`, `pb health`, `pb store`. Shouts at you with a help text, like a hungover `man` page.
- `config`: reads `PB__SECTION__KEY` env vars. Double underscores, on purpose. A convention discovered by pain.
- `rubato`: resamples audio. Turns 48,000 Hz into whatever the models want, no questions asked.
- `chromiumoxide`: drives Chromium for browser tests. A puppeteer for the puppeteer. Chromium is only invited for tests and has to wear a name tag.
- `rustix`: safe Unix calls, so we do not need `unsafe` to say hello to the kernel.
- `wasm-bindgen`, `js-sys`, `web-sys`: Rust and JavaScript talking to each other. They mostly argue. Rust wins on points, JavaScript on volume.
- `url`: parses URLs. A solved problem that is never solved.
- `fastrand`: picks your warning clip. The engine is not telling.

Honourable mention: `rustls-platform-verifier` and `webpki-root-certs`, which together are a very long guest list for the TLS party, and `futures`, which are things that will be done later. Like the README. Like the fan on your server.

## Cargo.lock Trivia

`Cargo.lock` has **1,079** entries (977 unique names), and 11,939 lines of other people's decisions. A few of them:

- 🏆 The longest name is `wgpu-core-deps-windows-linux-android`, 36 characters. A crate that is somehow *Windows, Linux, and Android at the same time*. A pure Frankenstein. 🧟
- 🪟 The lockfile contains `winapi`, `windows`, `windows-sys` and a small village of `windows_*_msvc` crates. They are lockfile guests. They are in the building. Nobody gave them a badge, and they will never be compiled on your Linux box. Scheiß Windows.
- 🎮 It also contains `cudarc` and `burn-cuda`. Same deal, no badge. (Fuck you, NVIDIA. See *Rants*.)
- 🍎 `objc2-*`: whole forest of Apple crates, also guests. Cupertino had a cocktail party and nobody showed up.
- 📚 Alphabetically first: `addr2line`. Last: `zune-jpeg`. In between: 975 reasons to pin your versions.

None of these are direct dependencies. They are friends of friends. We do not know them, but they live in `target/` and eat our disk. 🐜

## Git History Trivia

The main line before this README was commissioned has **25** commits.

- **19** by PaperTobi, **4** by Pacific6938, **2** by Claude.
- Weekday of every single commit: **Sunday**. We checked. `git blame` always answers "Sunday".
- The longest commit subject is 122 characters: *README: lots of unnecessary information, a FAQ, a glossary and short historical footnotes (the instructions are unchanged)*. Brevity is not a theme here.
- The average subject length is 72 characters. `git log --oneline` weeps.
- The very first commit is called "Initial commit". It is the most honest commit. It makes no promises, only `.gitignore`.
- The history contains `updated readme` four times and `Revert "updated readme"` twice. Pure commitment issues. It is the `ls`, `ls`, `ls` of Git. 🔄

## Docs Folder Trivia

The `docs/` folder contains real documentation (the architecture, the Fluxer API surface, the dependency choices, and the list of non-Rust exceptions). Here is a size chart anyway:

- **`docs/design.md`**: 260 — The architecture. The big picture.
- **`docs/fluxer-api.md`**: 88 — What the bot relies on from Fluxer.
- **`docs/dependencies.md`**: 76 — Why each dependency was chosen.
- **`docs/exceptions.toml`**: 103 — The few non-Rust pieces. A short list of sinners.

The `docs/proposals/` folder has four numbered design proposals: `0001-classifier-runtime`, `0002-vad-weights`, `0003-tts` and `0004-engine-actors`. Four proposals. No votes were held. Everyone just did it.

## Container Lore

The `Containerfile` builds the bot in three stages:

1. **`build`** (FROM `rust:1.99.0-bookworm`): compiles the bot and its web page. The loud stage.
2. **`weights`** (FROM `build`): downloads the models and voices, checks their SHA-256. The patient stage.
3. **final** (FROM `debian:bookworm-slim`): copies only what is needed. Runs as user `10001:10001`, exposes `8790`, starts `/opt/pb/bin/pb run`. The calm stage.

The quadlet units in `deploy/quadlet/`:

- **`profanity-watch.build`**: Builds the image from your checkout in `~/profanity-watch`
- **`profanity-watch-data.volume`**: The named volume that stores everything the bot keeps
- **`profanity-watch.container`**: Runs the bot, restarts it on failure, checks its health

The container runs read-only, drops all capabilities, forbids new privileges, and keeps its data in one volume. It is the most boring container in the world. This is a compliment.

## Windows, A Eulogy

🪟 *Gather round. We are here today to remember Windows support, which never existed.*

- **Windows support: none.** Not planned. Not even in this README's dream, and this README is *very* far gone.
- The Requirements section says "Linux". We meant it. It was not a typo. It was a statement of values.
- Scheiß Windows, as the locals say. Reasons, in no particular order:
  - the forced reboot in the middle of your 60-minute `cargo build`, with the message "Updating, 30%… 100%… 30%…";
  - `C:\Program Files (x86)\` with *two* spaces in the path, a feature designed to break every shell script ever written;
  - backslashes, as a path separator, on purpose, forever;
  - CRLF line endings, because one control character was too few;
  - the registry, which is a database of your sins;
  - the settings app, which is 40% settings and 60% advertisements for OneDrive.
- Windows has a Linux inside it now (WSL). Let that sink in. The operating system is so bad at running things that it shipped a *better* operating system in a box, like a sad IKEA. 🪑
- The 60-minute libwebrtc build is a pure Linux pain. Windows would make it worse, but it would try very hard.
- If you want to run this on Windows: use a VM. Or Podman. Or a different life.
- *Alt+F4*. For the nostalgics. 🔫

## C, The Loaded Gun

☢️ *A short, loving, deeply unfair introduction to the language that gave us everything and CVEs.*

- In C, every pointer is a hostage situation. `malloc` is a lottery, `free` is a prayer, and `strcpy` is a CVE with a function name.
- **Undefined behaviour** means the compiler is allowed to do anything. Reformat your disk. Launch the missiles. In the old joke, make *demons fly out of your nose*. The standards committee calls this "optimisation".
- **Use-after-free**: the zombie apocalypse of memory. You buried it, it came back, and now it has root.
- **Buffer overflow**: a 1988 classic, re-released every year like a vinyl.
- **Off-by-one**: in the old joke, one of the two hard problems in computer science, the other being cache invalidation and naming things. (Yes, that is three. That is the joke.)
- `Segmentation fault (core dumped)`: the only error message that is both a diagnosis and a threat. 💀
- The Rust borrow checker is a team of passive-aggressive senior engineers living inside your compiler. They reject your PR 14 times and then merge it with "nice work :)". 🦀
- **The deal in this repo:** the workspace sets `unsafe_code` to **deny**. Only `pb-espeak` (which talks to C, because Piper needs `espeak-ng` for phonemes) has `unsafe`. **7 lines** contain the word, with **4** `SAFETY` comments. Each of them is a tiny apology letter to the compiler. ✉️
- `cargo xtask ci` has a *zero-C gate*. The C stays in its cage. We feed it through a slot in the door. It is allowed out for libwebrtc and espeak-ng and nothing else, and `docs/exceptions.toml` is its parole file. 🚔
- C++ is also here, in libwebrtc. We do not discuss it. We have not discussed it since the 60 minutes. 🧊

## Segfault Support Group

🛋️ *Tuesday, 19:00, church basement, folding chairs. There is lukewarm coffee and a poster that says "`valgrind` is a state of mind".*

> **Dave (C, 22 years):** Hi, I'm Dave. It has been three days since my last segfault.
> **Everyone:** Hi, Dave.
> **Dave:** I was doing pointer arithmetic. Just a little. For fun. It was `arr[i+1]`. It was *always* `arr[i+1]`.
>
> **Gerda (C++):** Mine was a dangling reference. It looked *so* innocent.
>
> **Ferris (Rust):** I do not understand what you people are talking about.
> **Everyone:** …
> **Ferris:** I had a compile error once. It was very polite. It told me exactly what I did wrong and linked a page. I fixed it. That was it.
> **Dave:** Get out.
>
> **Pat (Python):** I do not have segfaults. I have `IndentationError`.
> **Gerda:** That is not a thing, Pat.
> **Pat:** It *is* a thing. It hurts. 🐍
>
> **Linus (kernel):** Please use `-Wall`. Please. I am begging. *Please.*
> *(Everyone stares at the floor. The coffee is cold now.)*

## The Unix Philosophy Tribunal

⚖️ *This repository stands accused of violating the Unix philosophy. The court is in session. The judge is `man`.*

- **"Do one thing and do it well."** The bot listens, scores, speaks, moderates, reports and hosts a web UI. 32 crates, one binary. *Guilty.* (Sentence: 3 years of `cargo xtask ci`.)
- **"Everything is a file."** Settings are `settings/*.toml`. Secrets are `secrets.toml` at mode `0600`. Logs are daily files. *Not guilty.* The court is moved.
- **"Silence is golden."** `pb run` prints plenty, but `pb health` prints almost nothing, which is how a health check should behave. *Acquitted on a technicality.*
- **"Worse is better."** The README is 2,000+ lines. *No further questions.*
- **"Make each program a filter."** The bot is a filter for swearing. *Not guilty.* Standing ovation.
- **`chmod 777`** solves all permission problems, says the intern. This repo uses `--read-only --cap-drop=ALL`. The intern has been escorted out.
- **`curl | sudo bash`**: the README does not ask you to do this. It asks you to wait 60 minutes and watch a compiler think. Which is worse is a matter of taste.
- **"Have you tried turning it off and on again?"** A `pb reset-setup` is the same, but with a hash chain. 🔌
- **`rm -rf`** on the data volume deletes the hash chain, the swear jar, and your friendships. The audit log cannot be edited. The *absence* of the audit log, however, is a form of editing. 🧑‍⚖️
- **YAML**: not here. TOML, which is INI with a lawyer. JSON, which has no comments and is proud of it. *Verdict: acceptable.*

## The Wisdom Of Stack Overflow

💬 *Collected answers from the Great Q&A, translated for this repository.*

- **Q:** How do I exit Vim? **A:** You don't. *(Closed as duplicate of "How do I exit Vim?")*
- **Q:** Why does the build take 60 minutes? **A:** Why would you want to do that?
- **Q:** My Rust code does not compile. **A:** *(accepted answer, 3,000 upvotes)* You are holding it wrong. Read the error. It is right. It is always right.
- **Q:** Can I make `pb` swear? **A:** Closed as off-topic. This is a *moderation* bot.
- **Q:** How do I run this on Windows? **A:** `sudo apt install linux`. *(comment: this is not a valid command)* *(reply: skill issue)*
- **Q:** I have a segfault in Rust. **A:** *(deleted)*
- **Q:** What is the best Linux distro for the bot? **A:** The one you will complain about. *(Arch users in the comments: "btw")*
- **Q:** Why is `unsafe` only in one crate? **A:** Because we locked it up. Please read the docs on `docs/exceptions.toml`. *(Marked as duplicate of a question about `goto`.)*
- **Q:** `rm -rf` hit the wrong folder. **A:** You will be fine. *(You will not be fine.)* *(Restore from your Podman volume backup. You did make one. Right? Right?)*

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

**Q: Does it run on Windows?**  
A: No. Scheiß Windows. See *Windows, A Eulogy*.

**Q: Can I use `curl | sudo bash` to install it?**  
A: There is nothing to `curl`. There is a `git clone`, a `podman build` and a deep breath. Roughly 60 minutes of deep breath.

**Q: Why does it use TOML and not YAML?**  
A: Because we like our indentation like we like our Wi-Fi: not part of the syntax.

**Q: Why not Docker?**  
A: Docker wants a daemon, and the daemon wants root, and root wants to ruin your Friday. Podman is the same, minus the "root wants to ruin your Friday".

**Q: Is this a fork bomb?**  
A: No. It is a very polite bot with a lot of crates. The crates are well behaved. Mostly.

**Q: Can I run it in Kubernetes?**  
A: You *can*. One bot, one container, one bad day. Do you really need a cluster to tell somebody to say "darn"?

**Q: Is the code `unsafe`?**  
A: One crate is, and it is locked in a cage. The cage has its own `SAFETY` comments, written by someone who was not sleeping. See *C, The Loaded Gun*.

**Q: Does the bot listen when I am not tracked?**  
A: No. Only tracked people, only their mics. It is not the NSA. It is a Sunday project.

**Q: Why is the README so long?**  
A: The first draft was 300 lines. Then it got hungry.

## Haikus About The Repository

**Haiku #1** 🌸

> A word is spoken  
> The classifier scores it  
> A clip says: please no

**Haiku #2** 🦀

> Thirty-two crates deep  
> The borrow checker is pleased  
> The build is not done

**Haiku #3** 🍵

> Port eight-seven-nine-oh  
> The web page is listening  
> Come in, have some tea

**Haiku #4** ⛓️

> Hash chain, hash chain, link  
> Each event remembers one  
> Nobody cheats here

**Haiku #5** 📢

> Keep it clean, he said  
> The voice is five seconds long  
> He swears even more

**Haiku #6** 🐳

> Podman starts the bot  
> Rootless and read-only now  
> No drama today

**Haiku #7** 🪜

> Strikes accumulate  
> A mute, a kick, a timeout  
> Peace in the channel

**Haiku #8** 🇩🇪

> Piper speaks in German  
> The words are polite and clear  
> The tone is not so

**Haiku #9** ☢️

> Unsafe stays in one place  
> The crate that talks to C  
> The rest is pure Rust

**Haiku #10** 🧊

> Silero hears breath  
> Is that a human or fridge  
> The fridge is quiet

**Haiku #11** 🌧️

> Sentences in queues  
> The classifier hums softly  
> Verdicts fall like rain

**Haiku #12** 🙈

> Observe only mode  
> The bot judges silently  
> A saint with a log

**Haiku #13** 🏋️

> Three gigabytes, yes  
> The weights are heavy, my friend  
> But the bot is light

**Haiku #14** 🔢

> Exit code seventy-eight  
> Read the log, dear friend  
> It tells you why

**Haiku #15** 🌞

> A Sunday commit  
> The weekend has been spent well  
> The tests are still red

**Haiku #16** 🪟

> Windows update, now  
> The build stops at ninety-nine  
> Penguin pours some tea

**Haiku #17** ☢️

> malloc, hope, and free  
> segmentation fault, core  
> dumped. Like my hopes.

**Haiku #18** 🧊

> Sixty minutes build  
> libwebrtc, libc++  
> The fan sings softly

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

> "I would like to speak to whoever approved 467 `unwrap()` calls."  
> — *Linus Torvalds (invented, would be very loud)*, ★★★☆☆

> "I have no idea what is happening, but there is no `unsafe` in my lane."  
> — *Haskell*, ★★★★☆

> "I am a segfault. I am not here. I am not in Rust. This is a safe place."  
> — *A Segfault*, ★☆☆☆☆

> "You compiled Chromium's standard library for a swear bot. We are so proud."  
> — *Google*, ★★★★★

> "I have been in /tmp since 2011. Nobody has ever read me. I accept my fate."  
> — *A Log File*, ★★★★★

> "Please use me. I am free. I am `vim`. I am not `emacs`. I am the better one."  
> — *The Vim Plugin That Wants To Be Your Friend*, ★★★★☆

> "Where is the Windows version?"  
> — *Nobody. Ever. In the history of this project.*, ★★★★★

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

- **Bot does not start**: `pb doctor`, then read the log
- **Exit code 78**: Read the log. It says why.
- **Exit code 3**: Another bot is using the same data directory. Stop it.
- **Page does not open**: `podman ps`, check the network and the firewall (`8790/tcp`)
- **Login fails**: Register the redirect address exactly in Fluxer
- **Record a clip greyed out**: The page needs HTTPS or localhost
- **Flagged but no warning**: Check strikes, observe only, deafened, Speak permission
- **Forgot setup code**: `pb setup-code`
- **Lost access**: `pb reset-setup`
- **Settings edited by hand**: Press *Read the settings files again* or send `SIGHUP`
- **Want to feel better**: Drink water

- **Segfault in Rust**: it is not a segfault, it is a lie. File an issue and a hat.
- **Want to run it on Windows**: close the window. 🪟

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
- [ ] Have you tried turning it off and on again?
- [ ] Is it a DNS issue? (It is always DNS.)
- [ ] Do not run `rm -rf` as a hobby.
- [ ] Touch grass. It does not compile, but it is free.
- [ ] Hydrate. Your build has been running for 47 minutes.
- [ ] Remember: Vim users can leave. They just do not.

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

## Rants, Hot Takes And Holy Wars

🐧 *Every developer has strong opinions about tools. The author has them too, and also a keyboard. Everything below is about software, never about people. Mostly.*

1. 🖕 **NVIDIA.** The optional GPU mode (`device = "gpu"`) goes through **Vulkan** (via wgpu), not CUDA. Fuck you, NVIDIA. 🎮 (A tip of the hat to the Finnish gentleman who said it first, on stage, in 2012.) `Cargo.lock` does contain `cudarc` and `burn-cuda`, as lockfile guests only. They are in the building, but nobody gave them a badge.
2. 💩 **GNOME.** This project has no GTK dependency. The web UI is a web page, on purpose, so nothing here will ever hide your settings "for simplicity" and then call the missing option a design decision. GNOME is shit. 🦶 (Not a technical statement, just a vibe. KDE people may now cheer. Then argue about Wayland.)
3. 🧵 **systemd.** `deploy/quadlet/` has three units, because even your containers now need an init system with opinions. systemd will eventually also read this README to you. And fix the typos. And restart itself.
4. 🐳 **Docker vs Podman.** Podman is Docker without the root daemon-shaped hole in your security. Rootless, read-only, `--cap-drop=ALL`. The most boring container in the world, and boring is the best thing a container can be. 😴
5. 🏔️ **Arch, btw.** The development section lists Arch, CachyOS and Manjaro first. Yes, on purpose. No, we are not sorry. By the way, we use Arch. 🧙
6. 🪨 **Debian stable.** The Containerfile installs clang 21 from LLVM's own repo because bookworm ships clang 14. Debian stable: stable like a rock, and just as up to date. 🦕
7. ⌨️ **Tabs vs spaces.** The Rust code has **0** tab characters. `rustfmt` ended that war, spaces won, and the tabs went home to write YAML. ☮️
8. 📝 **Vim vs Emacs vs nano.** The settings are edited in the page *or* by hand in TOML (`toml_edit` even keeps your comments, which is more than your colleagues do). We do not care what you use. We silently judge it. 🤐
9. 🐘 **Electron.** There is none. The web UI is Leptos compiled to WebAssembly: a small page, not a 400 MB Chrome in a trench coat. 🧥 Chromium is only invited for the browser tests, and it has to wear a name tag.
10. 🧊 **libc++ and libwebrtc.** LiveKit's libwebrtc is built against Chromium's own C++ standard library, so we have to build with clang 21 too. Google compiled its own standard library into a video-call stack and now *you* wait 60 minutes for it. Thanks. Thanks a lot. 🙃
11. 🏗️ **CMake.** Needed for espeak-ng. CMake is a build system the way a haunted house is a property investment. 👻
12. 📦 **npm.** `node_modules` folders in this repo: **0**. The `cargo xtask ci` has a *shipped-JavaScript gate* so nobody can smuggle in a `left-pad` at night. 🚫
13. 🐍 **Python.** The old bot was Python. `pip install --break-system-packages` are the three most honest words in Python packaging. 🪦 Indentation is syntax, and the interpreter has feelings about your spaces.
14. 🦀 **"Rewrite it in Rust."** Yes, we actually did that. The meme is now a commit history. The crab won. Ferris sends regards. ❤️
15. 🧅 **C.** The one `unsafe` crate (`pb-espeak`) talks to C. C: the language that gave us the whole of computing and the whole of CVE. ☢️ Locked in a crate. Not allowed out. We feed it through a slot in the door.
16. 🔐 **Rootless containers.** Because `sudo make me a sandwich` should at least need an actual sandwich. 🥪
17. 🍝 **Spaghetti.** There is none. There are 32 crates and a checker (`cargo xtask deps`) that yells if a lower layer so much as *looks* at a higher one. It is lasagne. The best kind of code. 🧀
18. 🕳️ **"It works on my machine."** The Containerfile is the formal answer: we shipped your machine. 🤷
19. 🤖 **AI.** Claude wrote a few commits in this repo and would like it noted that its code compiles on the first try. In Claude's imagination. 🧠💭
20. 📉 **Premature optimisation.** The Silero VAD rewrite made it ten times faster (0.2 ms → 0.02 ms per step, according to `docs/dependencies.md`). Maybe not premature. Maybe just *rude* to the old implementation. 🏎️

> 🧯 **The author's list of things that are *fine*:** Rust, Podman, `rustfmt`, the borrow checker, hash chains, Sunday commits, and anything that does not ask you to log in with a Google account to change a setting. 🤝

21. 🪟 **Windows.** Scheiß Windows. Forced updates, backslashes, CRLF, and an Event Viewer that is a novel by Kafka. Not supported, not planned, not even in this README's fever dream. Alt+F4 is a stage direction. 🔫
22. 🖥️ **Wayland vs X11.** The bot has no GUI, so neither of them can touch it. The web UI renders in a browser, in whatever you use, and if it breaks it is the *browser's* fault. Wayland users will explain this to you for 40 minutes. X11 users will explain it back. 🧑‍🤝‍🧑
23. 📦 **Snap.** Canonical's way of saying "your Firefox now takes nine seconds to start, and you will like it." The Containerfile uses Debian and `apt`, and it is *fine*. 🐢
24. 🏜️ **Gentoo.** Users are still compiling this README. They started in 2019. They will be done when the kernel is.
25. ❄️ **NixOS.** "Have you tried rewriting this in a flake?" No. No we have not. 🧊
26. ☸️ **Kubernetes.** One bot. One container. One bad day. We do not need a cluster, a service mesh and a Helm chart to say "keep it clean". YAML is a cry for help in a format.
27. 📜 **YAML.** TOML is INI with a lawyer. YAML is INI with a *cult*. Yes, `no` is `false`. Yes, Norway is `false`. Yes, we are still mad. 🇳🇴
28. ✍️ **Emacs.** A great operating system lacking a good text editor. Fight me, in `M-x doctor`.
29. 🔥 **Zero-day.** The most expensive way to find out your `unsafe` was not safe. ☢️
30. 📟 **The OOM killer.** The kernel's way of saying "no". Exit code `137` knows him personally. 🔪

## More Real Numbers

📏 *We counted again. Every number below is real and was measured on the code in this branch (`crates/` and `xtask/`). We should seek help.*

- **Lines of Rust (`crates/` + `xtask/`)**: 50,572 — More than the README. For now. 📈
- **Blank lines**: 3,603 (7.1%) — Breathing room. 🌬️
- **Comment-only lines**: 3,028 (6.0%) — The author talks to themselves. 🗣️
- **Tab characters**: **0** — `rustfmt` won the holy war. ☮️
- **Lines longer than 100 characters**: 1,593 — `rustfmt.toml` says `max_width = 120`. We live at the edge. 🏔️
- **Lines longer than 120 characters**: 159 — Long strings and macros that `rustfmt` refuses to wrap. Not a bug. A personality. 🎭
- **Longest line**: 196 characters — `crates/pb/tests/browser.rs`. Browser tests: where lines go to be long. 🌐
- **`fn` definitions**: 2,131 — One for every 24 lines. 🧮
- **Most common function name**: `fmt` (59) — Everybody wants to be printed nicely. 💅
- **Runner-up names**: `new` (44), `start` (27), `default` (27), `run` (24) — Constructors, defaults and hope. 🌱
- **Lines with `impl`**: 519 — The crab implements things. 🦀
- **Lines with `match`**: 595 — Enums are not left out after all. 🥳
- **Lines with `.await`**: 1,402 — About 2.8% of the code is the bot waiting politely. ⏳
- **Lines with `Option<`**: 623 — Optimism. 😊
- **Lines with `Result<`**: 463 — Realism. 😐
- **`#[derive` lines**: 466 — Why write code when the compiler writes it for you. 🪄
- **`#[test]` / `#[tokio::test]`**: 153 / 85 — The test suite has more async than most people's lives. 🧪
- **`'static`**: 90 lines — The lifetime of all lifetimes: forever. ♾️
- **`Box<dyn`**: 28 lines — Dynamic dispatch, used sparingly, like salt and trust. 🧂
- **`panic!` / `unreachable!`**: 50 / 22 — "This can never happen" is a lovely way to start a bug report. 💥
- **Lines containing `unsafe`**: 7 — All in the smoking area. 🚬
- **`SAFETY` comments**: 4 — Each one is a tiny apology letter to the compiler. ✉️
- **The word "please" in the Rust code**: 6 — Polite. 🙏
- **The word "sorry" in the Rust code**: 0 — Confident. 😎
- **Swear words in the repository (outside this README)**: **0** — A profanity bot with a spotless mouth. We checked for `fuck`, `shit`, `damn`, `bitch`, `asshole` and `crap`. This README has ruined that. 😈
- **Most common word in the doc comments**: `voice` (226) — Then `every` (154), `fluxer` (132), `community` (114), `settings` (105). A bot with a clear theme. 🎙️
- **Files tracked by git**: 447 — Among them 235 Rust files in `crates/` and 5 `.wav` files that make up a very small choir. 🎼
- **Markdown files (without `third_party/`)**: 9 — And this README is the loudest one. 📢
- **TOML files (without `third_party/`)**: 42 — TOML: INI with a lawyer. 📋
- **`Cargo.lock`**: 11,939 lines — Eleven thousand lines of other people's decisions. 🧾
- **`third_party/`**: 2.0 MB — Four vendored crates, lovingly patched. 🩹
- **Ignored security advisories in `deny.toml`**: 4 — Each with a written excuse and a way out. A polite shrug, in TOML. 🤷

🏁 **The grand total of things in this table that anybody needed:** zero. **The grand total of things in this table that are true:** all of them.

## Mailing List Mode

📧 *A fictional code review, as if this repository were sent to a very grumpy kernel-style maintainer. The maintainer is invented. Any resemblance to a famous Finn who yells at graphics vendors is, of course, a coincidence. 🙄*

```
From: Grumpy Maintainer <grumpy@localhost>
To: PaperTobi <you-know-who@example.invalid>
Subject: Re: [PATCH v7 000/235] README: even more nonsense

On Sunday, the author wrote:
> This README is mostly filler.

NAK. "Mostly" is doing way too much work in that sentence. 😤

> 32 crates behind versioned (v1) interfaces.

Why the hell are there 32 crates for a bot that tells people to stop
swearing?! ... Oh. There is a layer checker. And it fails the build when
somebody breaks the rules. Fine. I hate how much I like that. 🧀

> `unwrap()` appears 467 times.

Who the *hell* ... tests. They are tests. Clippy says tests may unwrap.
A test fails by panicking. I have no argument. This is annoying. 😠

> The `branches` crate had to be patched because nightly renamed an intrinsic.

A vendored crate called `branches` in a git repo with branches. Whoever
named this: you win today. Do not let it go to your head. 🌿

> LiveKit's libwebrtc is built against Chromium's libc++, so we need clang 21.

And the Debian image ships clang 14. Of course it does. Whoever decided
that was "stable" should be forced to write code on it, for a week,
without autocomplete. 🪨

> The `unsafe` is confined to `pb-espeak`.

Good. Keep that animal in its cage and do NOT feed it after midnight. ☢️

> A hash-chained event log, for a bot that tells people off.

This is the most overengineered thing I have read this week and I am
slightly in awe. Applied. ⛓️

> The VAD was kicked out of the Burn club; the hand-written loops are 10x faster.

See? You can just *write the loop*. Not everything needs a framework with a
logo. Plus one. 🔁

> `updated readme`, `Revert "updated readme"`, `updated readme`, ...

Commit messages like this are a cry for help. I also see "Revert". Twice.
Make up your mind, for fuck's sake. 🫠

> The README now contains swearing, aimed at software.

My workstation is fine, thank you. The bot, however, is going to flag this
whole mail. 🚨 ... Good. Serves it right.

Overall: not a single sensible change, a hundred pointless ones, and I
am applying it all. Just never send me a "v8". 🙏

Signed-off-by: Grumpy Maintainer <grumpy@localhost>
Reviewed-by: The Borrow Checker (after 14 attempts) 🦀
Acked-by: Clippy 📎 (with 3 warnings, which it will mention)
Nacked-by: The Wombat 🐨 (on principle)
Tested-by: Nobody 🤷 (Sunday)
```

## Insider Gags Glossary

🤓 *Jokes only developers get. If you do not get them, congratulations, you have a social life. 🌞*

- ****LGTM** 👍**: "Looks good to me" — Said by people who read the title, not the diff.
- ****Bikeshedding** 🚲**: Arguing about trivial things — Entire teams will argue about the colour of the settings page. Nobody reads the escalation ladder.
- ****Yak shaving** 🐃**: A chain of tasks nobody asked for — To tell a teenager off for swearing, we first had to learn what Chromium's libc++ is.
- ****Heisenbug** 🫥**: A bug that vanishes when observed — `Observe only (silent)` mode, but for bugs.
- ****PEBKAC** 🧑‍💻**: The problem is between keyboard and chair — See the Troubleshooting table. Exit code 78 is often this.
- ****RTFM** 📖**: Read the manual — See: *read the log*. The log is the manual now.
- ****Bus factor** 🚌**: How many people can get hit by a bus before the project dies — Between 1 and 1.5. The 0.5 is an AI.
- ****Tech debt** 💸**: Shortcuts you pay for later — `deny.toml` lists four of them, with receipts.
- ****Cargo cult** 🛐**: Copying rituals without understanding — 1,079 packages in `Cargo.lock`. We understand about 32.
- ****Dogfooding** 🐶**: Using your own product — The README would fail its own model. We eat our own swearing.
- ****XY problem** 🔀**: Asking for Y when you need X — "How do I make the bot swear?" You do not. See the FAQ.
- ****Works on my machine** 💻**: The universal excuse — The Containerfile is what happens when the excuse wins.
- ****Premature optimisation** 🐎**: Root of all evil (Knuth) — Silero is ten times faster now. Mature optimisation. Still rude to the old code.
- ****Cargo.lock regret** 🔒**: A file you never read and always commit — 11,939 lines of other people's decisions.
- ****Git blame** 🔦**: Find who wrote it — The answer is "Sunday". Every time.
- ****Force push** 💣**: Rewriting shared history — The git log shows `Revert` commits instead. Like adults. Twice.
- ****Friday deploy** 🔥**: A career-limiting move — The Wellness Checklist forbids it. Read it.
- ****Nightly Rust** 🌙**: The unstable compiler — The code stays within stable Rust. Nightly is a lifestyle choice.
- ****RIIR** 🦀**: "Rewrite it in Rust" — Done. It is in the git log.
- ****I use Arch, btw** 🏔️**: Mandatory announcement — Listed first in the dev instructions. We are not sorry.

- **Segfault** 💥: the kernel gently telling you that you touched memory that is not yours. Rust says "no" before it happens. C says "yes" and then calls the police.
- **Undefined behaviour** ☢️: the compiler is allowed to do anything. Nasal demons. Hence `unsafe` is in a cage.
- **OOM killer** 🔪: the kernel's bouncer. See: exit code 137.
- **`sudo`** 🧙: "I am the root and I know what I am doing." You do not.
- **`chmod 777`** 🔓: the intern's answer to every permission problem. This repo uses `--cap-drop=ALL`.
- **Kernel panic** 😱: the Linux version of a blue screen, but with dignity and a stack trace.
- **Gentoo user** 🏜️: a person who says "just compile it" and means it.
- **`/dev/null`** 🕳️: where your hope goes. Also, where `Observe only (silent)` would send the log, if it were not so judgemental.
- **Bikeshedding** 🚲: see above. Tabs vs spaces: we never speak of it.
- **Hello world** 👋: the first program anybody writes. This repo's `hello world` is 49,581 lines. Do not ask.

## Developer Bingo

🎯 *Mark every square that applies to this repository. Five in a row means you have been in this industry too long.*

- 🦀 Rewritten in Rust · 🧵 Mentions systemd · 🏔️ "I use Arch btw" · 🐳 Container instead of fixing it · 🪦 Predecessor in a different language
- 🧪 Tests that unwrap · 🔥 Build takes 60 min · 🌙 Needs nightly "but doesn't really" · 🍝 "No spaghetti" (it is lasagne) · 📜 README longer than the manual
- 🧊 Vendored dependency with a patch · 🔒 Lockfile bigger than the code is tall · ⭐ FREE SPACE (the wombat) · 🌞 Everything committed on a weekend · 🕳️ Mentions libc++ in anger
- 🤖 An AI wrote a commit · 🚪 Special exit code with lore · 📉 "We optimised 10x" · ⛓️ Hash chain for no good reason · 😬 467 unwraps
- 🖕 Rants at a GPU vendor · 💩 Insults a desktop environment · 🪨 Debian stable joke · 🎙️ Swearing in a profanity bot · 🔁 Reverts its own "updated readme"

*A full house is called "the actual state of this repository". 🏠*

## The Man Page

📜 *In the long tradition of man pages nobody reads. `man pb` does not exist. This is its spirit.*

```
PB(1)                       Profanity Watch Manual                       PB(1)

NAME
       pb - listens to your friends so you do not have to 🎙️

SYNOPSIS
       pb [run | doctor | health | setup-code | reset-setup |
           fetch-weights | import | store | settings]

DESCRIPTION
       pb follows chosen people into Fluxer voice calls, scores every
       sentence with a model, and tells them off with a recorded voice when
       they get creative. It runs on a CPU. It does not need a GPU. It does
       not want a GPU. It especially does not want CUDA. 🖕

OPTIONS
       --help   Prints help, and a mild sense of failure.
       --dest   Where the 1.7 GB of weights land. Patience required. ⏳

EXIT STATUS
       0    Clean exit. Suspicious.
       3    Another bot already uses this data directory. 🐺
       78   Read the log. It tells you why. (EX_CONFIG, BSD, 1980s.) 🧙
       137  Killed. Probably memory. Probably you.

ENVIRONMENT
       PB_DATA                where everything lives
       PB__SECTION__KEY       config, with deliberately double underscores
       PB_BOT_TOKEN           the token. Do NOT paste it into a chat. 🥷

FILES
       $PB_DATA/config.toml   how the process runs
       $PB_DATA/settings/     TOML, with your comments preserved. 💕
       $PB_DATA/secrets.toml  mode 0600, as the gods intended.

BUGS
       Yes. 🐛

AUTHORS
       PaperTobi, Pacific6938, Claude. All on a Sunday.

SEE ALSO
       podman(1), systemd(1) (sigh), rustc(1), your therapist(7), sleep(1)

Linux                              Sunday                               PB(1)
```

## Compiler Error Therapy

🛋️ *The Rust compiler is the only code reviewer that is never wrong, never tired and never says "LGTM" without reading. Here is what its messages really mean.*

- **`E0382`**: "borrow of moved value" — You gave it away. You cannot also keep it. Life lesson. 💔
- **`E0499`**: "cannot borrow as mutable more than once" — Two people cannot hold the same pen. Share, or go to jail. ✒️
- **`E0502`**: "cannot borrow as mutable because it is also borrowed as immutable" — Someone is reading it. Stop editing it. 📖
- **`E0308`**: "mismatched types" — You said number. You meant text. Both of us know it. 🔢➡️🔤
- **`E0277`**: "the trait bound is not satisfied" — You need to prove you are who you say you are. 🪪
- **`E0425`**: "cannot find value in this scope" — It exists. Just not here. Like your motivation on Mondays. 🫥
- **`E0599`**: "no method named ... found" — You made that up. We both know. Like your estimate. 🤥
- **`lifetime may not live long enough`**: (a long paragraph) — Ask your parents. 👪
- **`unused variable`**: warning — You wrote it and never used it. Like the gym membership. 🏋️

> 🧘 **Five stages of grief, Rust edition:** denial (`I'm sure it compiles`), anger (`E0382`), bargaining (`.clone()` everywhere), depression (`Arc<Mutex<Box<dyn Trait>>>`), acceptance (`unsafe`, then a long walk). 🚶

- `E0133`: *"call to unsafe function requires unsafe block"*. The compiler asks you to sign a waiver. 📝
- `Segmentation fault (core dumped)`: *this is not a Rust error, this is a rumour*. 🦀

## Commit Message Hall Of Fame

🏆 *Real commit messages from the history of this repository. We did not invent these. We could not. 🙈*

- **`Initial commit`**: The most honest commit. It makes no promises. 🤞
- **`updated readme` (four times)**: The man did not trust the first one. 📜
- **`Revert "updated readme"` (twice)**: And then he did not trust the revert. 🔄
- **`Update .gitignore and .containerignore: secrets, editor/OS files, sqlite and toolchain leftovers`**: Prevention of future tears. 😭➡️🙂
- **`Finish replacing the real user ID in tests`**: "Finish." Implies there was a *start*. 🕵️
- **`Engine: supervised actors with kept mailboxes, health in /healthz and on the System page`**: When a commit message is longer than the average attention span. 🧠💨
- **`Silero VAD: hand-written forward pass instead of Burn`**: "Instead of a framework, a for loop." The oldest trick in the book. 🔁
- **`Build with Rust nightly: toolchain file, patched branches for turso, image and README`**: "Patched branches". In a repo with branches. 🌿
- **`README: lots of unnecessary information, a FAQ, a glossary and short historical footnotes (the instructions are unchanged)`**: A hundred and twenty-two characters of honesty. 🫡
- **`README: remove the trivia, FAQ, glossary and footnotes`**: And then, naturally... 😏
- **`updated readme` (this very change)**: The tradition lives on. 🕯️

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

The 49 setting keys, in alphabetical order, in a code block, because a table would summon the author.

```
setting-actions-enabled
setting-admin-role-ids
setting-admin-user-ids
setting-admins-play-audio
setting-allow-e2ee-downgrade
setting-allowed-hosts
setting-announce-actions
setting-audience
setting-chat-language
setting-command-prefix
setting-commands-enabled
setting-cpu-threads
setting-digest
setting-digest-time
setting-digest-weekday
setting-end-silence
setting-escalation
setting-fallback-languages
setting-greet-enabled
setting-guild-allowlist
setting-instance
setting-jar-enabled
setting-join-settle
setting-label-enabled
setting-label-threshold
setting-leave-grace
setting-max-reaction-delay
setting-max-sentence
setting-min-voiced
setting-modlog-audio
setting-modlog-channel
setting-no-speak-policy
setting-observe-only
setting-owner-dm-audio
setting-paused
setting-recordings
setting-speech-rate
setting-strike-notice
setting-strike-window
setting-strikes
setting-threshold
setting-timezone
setting-tracked-everywhere
setting-tts-threads
setting-tts-voices
setting-ui-url
setting-violation-window
setting-voice-language
setting-volume-db
```

## Appendix E: Numbers That Appear In The Docs

- **0.6**: default threshold
- **8790**: web port
- **3**: exit code: another bot uses the volume
- **78**: exit code: permanent problem
- **12**: hours an owner stays logged in
- **7**: days an admin stays logged in
- **15**: minutes that count as a recent login for secret changes
- **365.25**: maximum time-out length in days
- **1.7**: GB of models and voices
- **3**: GB of RAM at peak
- **8**: GB of RAM to build
- **25**: GB of disk to build
- **30–60**: minutes for the first build
- **4.4**: minimum Podman version
- **5.2**: Podman version for the quadlet units
- **21**: minimum clang version
- **10001**: uid of the user inside the container
- **0600**: mode of `secrets.toml`

## Licences

The bot: AGPL-3.0-or-later (`LICENSE`); if you run a changed version for others, offer them its source (the web
page links to it). The Roblox voice-safety classifier: Roblox's model licence (next to the weights); Silero VAD: MIT;
Piper voices: see their model cards (Thorsten-Voice: CC0; lessac: the Blizzard 2013 licence); espeak-ng:
GPL-3.0-or-later; LiveKit's libwebrtc: BSD-3-Clause.

---

*This README is mostly filler. The filler is, at least, about this repository. The wombat has been dismissed. Then rehired.* 🐨

🐧 *Linux, Rust, Podman and a very long README: the four pillars of this repository. The fifth pillar is the Sunday. The sixth is the wombat. The seventh is the fan on your server, which is still running at 100% and has asked to speak to a manager.* 🌀
