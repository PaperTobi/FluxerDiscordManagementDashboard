# Profanity Watch

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

## Requirements

| | |
|---|---|
| System | Linux. For the container: **Podman 4.4+** (rootless is fine; 5.2+ for the systemd units in `deploy/quadlet/`) |
| CPU | x86-64 with AVX2, AES and BMI2 (most CPUs since about 2014), or ARMv8 with the crypto extensions |
| Memory | about **3 GB** at peak while running; building needs about 8 GB and 25 GB of disk |
| Network | outgoing internet including **UDP** (voice); **TCP 8790** reachable in your network for the web page |
| Fluxer | a bot application, and someone with **Manage community** or **Administrator** to invite it |

Run **one** bot per bot token.

## 1. Create the bot in Fluxer

1. In Fluxer open **User Settings → Applications**, create an application and copy its **Bot token**
   (`<application id>.<secret>`) and its **Client secret**. Keep both private; if they leak, reset them there.
2. Tell the people you will track that the bot listens to them (see *Privacy and data*).

The bot is invited to your community after the setup (step 3), with a link from its web page.

## 2. Run it

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

Import the old data directory into a new, empty volume before the first start:

```bash
podman run --rm -v profanity-watch-data:/data:U -v proofanitybot-data:/old:ro profanity-watch import --from /old
```

Settings, tracked people, voice-line texts and clips, history and violations, recordings, the audit trail, swear-jar
counts, pending timed mutes, installed voices and the secrets are carried over; `import-report.txt` lists what was done
and which old settings no longer exist (the old caps).

## Troubleshooting

| What you see | What to do |
|---|---|
| The page does not open | container running (`podman ps`)? same network? firewall (`8790/tcp`)? open it by IP |
| "This address is not one of the bot's web UI addresses" | open it by IP, then set *System → Web UI address* or *Extra host names* |
| Login fails at Fluxer | the redirect address is not registered exactly (setup step 4; the login names the address), or the client secret is wrong (enter it again on the setup's last step, or *System → Client secret*) |
| "Record a clip" is greyed out | the page is not opened over HTTPS (see *HTTPS*) or on localhost; upload a file instead |
| Exit code 78 | the log says why (configuration, model files, CPU); `pb doctor` checks everything |
| Exit code 3 | another bot process uses the same data directory |
| `rustc: symbol lookup error: …librustc_driver….so: undefined symbol …` | the distribution's Rust package does not match its LLVM libraries (a partial update, or packages from different repositories): install rustup instead (*Development*) |
| `rustup could not choose a version of cargo to run` | `rustup default stable`, then in the project directory `rustup toolchain install` |
| `target/release/pb`: unknown command / no such file | the build before it failed: scroll up to its first error |
| *System* says the instance has voice turned off | that Fluxer instance has no voice calls; nothing for the bot to do there |
| Does not join voice | person not tracked or paused, missing Connect, or an end-to-end encrypted call (setting *Join end-to-end encrypted calls*) |
| Flagged but no warning | strikes not reached yet, *Observe only (silent)* is on, the person is deafened, or the bot may not speak (it writes in the chat instead) |

## Privacy and data

Audio is processed in memory, on your machine. A sentence's recording is kept only when it was **flagged** (the owner
can switch *Recordings* to every sentence or to none); recordings stay until the owner deletes them (person page →
Recordings). Scores, decisions and every change are kept in an append-only, hash-chained event log in the data
directory; nothing is deleted automatically. Recordings go to the owner's direct messages and, if enabled, to the
mod-log channel. The bot is visible in the call while it listens.

**Recording or analysing people's voices may need their consent where you live: tell the people you track.**

## Development

Tools: Rust through [rustup](https://rustup.rs) (the release in `rust-toolchain.toml`, with the `wasm32-unknown-unknown`
target for the browser bundle), clang 21+ and lld (LiveKit's libwebrtc is built against Chromium's libc++), glib
headers and pkg-config (libwebrtc), cmake and ninja (espeak-ng), git, and for the checks
`cargo install --locked cargo-deny cargo-shear`.

```bash
# Arch, CachyOS, Manjaro (rustup replaces the distribution's `rust` package)
sudo pacman -S --needed base-devel rustup clang lld pkgconf glib2 cmake ninja git
# Debian, Ubuntu: rustup from https://rustup.rs; clang 21 from https://apt.llvm.org (the Containerfile's build stage
# lists the packages)

rustup default stable        # a Rust for everything else (without it: "rustup could not choose a version")
rustup toolchain install     # in this directory: the release and target from rust-toolchain.toml
```

A distribution's own Rust package works only if it is that release, has the wasm32 target and matches the system's
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

## Licences

The bot: AGPL-3.0-or-later (`LICENSE`); if you run a changed version for others, offer them its source (the web
page links to it). The Roblox voice-safety classifier: Roblox's model licence (next to the weights); Silero VAD: MIT;
Piper voices: see their model cards (Thorsten-Voice: CC0; lessac: the Blizzard 2013 licence); espeak-ng:
GPL-3.0-or-later; LiveKit's libwebrtc: BSD-3-Clause.
