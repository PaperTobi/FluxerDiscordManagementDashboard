# The Fluxer API as the bot uses it

Verified 2026-10-04 against the official docs (https://docs.fluxer.app, built from `fluxer_docs/` in
github.com/fluxerapp/fluxer at commit `f3c777b2`) and, where the docs are silent, the server source at that commit.
`pb-fluxer` implements exactly this; `pb-fluxer-fake` simulates it. Re-check on upgrades.

## Discovery
`GET {origin}/.well-known/fluxer` (no `/v1`, no auth, 60/min). `endpoints.api_public` is the REST origin for bots
(`https://api.fluxer.app`), `endpoints.gateway` the websocket (`wss://gateway.fluxer.app`), `endpoints.media` the CDN.
Published schemes and ports are used as given. Also `api_code_version` (1), `features.voice_enabled`, `limits`.

## Gateway
- `{gateway}/?v=1&encoding=json` (no compression). Client frames ≤ 4096 bytes.
- Opcodes: 0 Dispatch, 1 Heartbeat (both ways), 2 Identify, 3 Presence Update, 4 Voice State Update, 6 Resume,
  7 Reconnect, 8 Request Guild Members, 9 Invalid Session, 10 Hello, 11 Heartbeat ACK, 14/15/16 client requests the bot
  does not use. Unknown opcodes: log and ignore.
- Hello `{heartbeat_interval}`. Heartbeat `d` = last sequence or null. A server op 1 is answered at once. No ACK for
  45 s → the server closes 4009; the client treats a missing ACK before its next beat as a dead connection.
- Identify `{token (raw), properties{os, browser, device}, presence?, ignored_events? (≤256)}`; no intents. A held
  Identify is retried silently by the server; an over-budget one is dropped (the client resends after a while).
- READY `{session_id, version, user, users, guilds[{id, unavailable:true}], …}`, then one GUILD_CREATE / GUILD_DELETE
  per guild. In those GUILD_CREATEs a member's `user` is only `{id}`; the full users are READY's `users` (a GUILD_CREATE
  later, when the bot joins a community, carries them in full).
- Resume `{token, session_id, seq}` on the same URL within 60 s; the server replays, then `RESUMED`.
  Op 7 → the server closes 4000 → resume. Op 9 is always `d:false`: identify again on the same socket.
- Close codes: 4000 resumable; 4001/4002/4005/4008/4009 reconnect (resume when possible); 4003 identify; 4004 on
  Identify = bad token (fatal: a UI state, not a crash), on Resume = identify; 4007 identify; 4010, 4011 (sharding needed
  above 2,500 guilds), 4012 fatal.
- Limits: 600 frames / 60 s per socket; op 3 at most 5 per rolling 20 s (more are dropped silently, so the client
  sends the latest presence no faster than that); op 4: 2 per second immediately, the rest queued server-side one per
  500 ms (a newer one replaces a queued one for the same connection).

## Guilds, voice states, messages (dispatches)
- GUILD_CREATE: `id`, `properties` (incl. `name`, `owner_id`), complete `roles`, `channels` (visible to the bot),
  `voice_states`, `members` (the bot and voice participants). Replace stored lists.
- GUILD_UPDATE (no roles), GUILD_DELETE `{id, unavailable?}` (unavailable → keep as unavailable),
  GUILD_ROLE_CREATE/UPDATE `{guild_id, role}`, GUILD_ROLE_UPDATE_BULK `{guild_id, roles}`, GUILD_ROLE_DELETE
  `{guild_id, role_id}`, CHANNEL_CREATE/UPDATE/DELETE, CHANNEL_UPDATE_BULK `{guild_id, channels}`,
  GUILD_MEMBER_ADD/UPDATE (member + `guild_id`), GUILD_MEMBER_REMOVE `{guild_id, user{id}}`.
- VOICE_STATE_UPDATE: `guild_id, channel_id (null = left), user_id, connection_id, session_id, member, mute, deaf,
  self_mute, self_deaf, self_video, self_stream, suppress, e2ee_capable, version`.
- VOICE_SERVER_UPDATE: `token, endpoint (ws/wss), connection_id, channel_id, guild_id?, e2ee_key?`. The grant lives 600 s.
- Op 8 Request Guild Members `{guild_id, query?, limit? (≤ 100), user_ids? (≤ 100), presences?, nonce? (≤ 32 bytes)}`:
  bots may ask one community per request; answers come as GUILD_MEMBERS_CHUNK `{guild_id, members, chunk_index,
  chunk_count, nonce?}` (`guild_request_members*.erl`). This is how the web UI searches members to add.
- MESSAGE_CREATE: message + `guild_id?`, `member?{roles,…}` (no `user`), `author{id, bot?}`, `webhook_id?`.

## Joining voice
Op 4 `{guild_id, channel_id, connection_id?, self_mute, self_deaf, self_video, self_stream}`: without
`connection_id` a new connection; with it, update or move that one; leaving needs `connection_id`. Refusals send
nothing. A join is pending for 30 s and becomes active when the bot joins the LiveKit room (or a second op 4 with the same
channel and connection) [source: `fluxer_gateway/src/guild/voice/guild_voice_connection_join.erl`,
`guild_voice_connection_confirm.erl`]. LiveKit room `guild_{g}_channel_{c}`, participant identity
`user_{uid}_{connection_id}`. Without SPEAK the bot is admitted suppressed. Bots may join end-to-end encrypted channels
and switch E2EE off for the channel. Fluxer announced (2026-09-28) a QUIC transport with DAVE E2EE to replace LiveKit,
no date; `pb-voice-api` keeps the transport replaceable.

## REST
`{api_public}/v1`, `Authorization: Bot <app_id>.<secret>`. Global 50 requests/s; 429
`{code:"RATE_LIMITED", global, retry_after (s, fractional)}`; headers `Retry-After`, `X-RateLimit-Bucket`, `-Limit`,
`-Remaining`, `-Reset-After`, `-Global`; 503 with `Retry-After: 1` when overloaded.
- `GET /applications/@me` (owner), `GET /users/@me`.
- `POST /channels/{c}/messages` (20 per 10 s per channel): `content` (bots: up to 4000 characters), `message_reference`,
  `allowed_mentions` (`{}` = none), multipart `payload_json` + `files[N]` with `attachments[{id:N, filename}]`, ≤ 10 files
  (a bot counts as premium: 500 MiB each).
- `PUT /channels/{c}/messages/{m}/reactions/{emoji}/@me` (URL-encoded emoji).
- `POST /users/@me/channels {recipient_id}` always succeeds; a refused DM fails when sent: 400
  `CANNOT_SEND_MESSAGES_TO_USER`.
- `GET/PATCH /guilds/{g}/members/{u}`: `mute` (MUTE_MEMBERS), `deaf` (DEAFEN_MEMBERS), `channel_id: null` = disconnect
  (MOVE_MEMBERS; 400 `USER_NOT_IN_VOICE` when not in voice), `communication_disabled_until` (MODERATE_MEMBERS; at most
  365.25 days; not on administrators; 400 `TWO_FACTOR_REQUIRED` in guilds with elevated MFA), each needing a role above the
  target.
- `GET /guilds/{g}/channels`, `GET /guilds/{g}/roles`.

## Permissions
ADMINISTRATOR 1<<3, MANAGE_GUILD 1<<5, ADD_REACTIONS 1<<6, STREAM 1<<9, VIEW_CHANNEL 1<<10, SEND_MESSAGES 1<<11,
ATTACH_FILES 1<<15, READ_MESSAGE_HISTORY 1<<16, CONNECT 1<<20, SPEAK 1<<21, MUTE_MEMBERS 1<<22, DEAFEN_MEMBERS 1<<23,
MOVE_MEMBERS 1<<24, MODERATE_MEMBERS 1<<40 (masks are decimal strings). Owner = all; else @everyone (role id = guild id)
OR assigned roles, ADMINISTRATOR = all; channel overwrites: @everyone, then the member's roles together, then the member
(each deny then allow).

## OAuth2 (web login)
`GET /v1/oauth2/authorize` (PKCE S256), `POST /v1/oauth2/token` (form; `authorization_code`, `refresh_token`),
`GET /v1/oauth2/userinfo` (Bearer, scope `identify`) → `{id, username, global_name, avatar, …}`,
`POST /v1/oauth2/token/revoke`. `redirect_uri` must match a registered one exactly; codes live 10 min.
The API's `GET /v1/oauth2/authorize` redirects the browser to the web app's consent page (`OAuth2Controller.ts`), so the
API endpoint alone is enough to start a login. A bot token is `<application id>.<secret>` (`BotAuthService.parseBotToken`);
the application id is the OAuth2 client id, so a web login needs no gateway connection.
