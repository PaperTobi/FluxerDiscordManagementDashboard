# 0009 — A documented read API, webhooks, and a leaderboard that runs elsewhere

Date: 2026-10-05. Status: planned (the user chose "webhooks / API" in the quiz of 2026-10-04: "so the leaderboard can be
hosted somewhere else … everything should be separated so it has proper apis that could just swap something else in
that uses the same apis, proper api docs … all in rust").

## What it is
- **A read API** at `/api/v1` on the bot's web server: communities, leaderboards (the swear jar), violations (in calls
  and in chat), a person's counts, and totals over time. JSON, documented by an OpenAPI 3.1 document at
  `/api/v1/openapi.json` and a page in the web UI that renders it. Read-only: changing the bot stays in the web UI and
  the chat commands.
- **API tokens**, made by the bot owner (System → API): a name, what it may read, optionally only some communities.
  The token is shown once; the bot keeps only its SHA-256. Requests send `Authorization: Bearer pbk_…`.
- **Webhooks**: the owner registers an HTTPS address, a secret and the kinds of events it wants (violations, actions,
  flagged chat messages, joins and leaves). The bot POSTs each event as JSON with `X-PB-Signature: sha256=<HMAC of the
  body>` and `X-PB-Delivery: <event number>`. Delivery is at least once and in order: each webhook keeps a cursor into
  the event log that only moves on a 2xx answer; failures are retried with growing waits (up to an hour between tries,
  never given up), so a receiver that was down catches up from where it stopped.
- **`pb-leaderboard`**: a separate small program in this repository (Rust, server-rendered HTML, no browser code) that
  reads the API with a token and serves a public leaderboard page for chosen communities. It is an example of
  "something else that uses the same APIs": it knows the bot only through `pb-api-proto`.

## What a token may read (scopes)
| scope | gives |
|---|---|
| `leaderboard` | communities' names and icons, people's shown names, avatars and swear-jar counts |
| `violations` | violations with time, person, type, step and decision (no recordings, no chat text) |
| `details` | also the text of flagged chat messages and of transcripts (when speech recognition is on) |
| `stats` | totals per community and day |

## Crates
- `pb-api-proto` (L0, pure, wasm-safe): the v1 request and response types (serde), the scope and event-kind names, the
  signature format, the OpenAPI document's schemas (`utoipa` derives, behind a feature so clients need not compile it).
- `pb-api` (L4): the axum router for `/api/v1` (token check, scopes, cursors and pages), the OpenAPI document, and the
  webhook dispatcher (one task per webhook, following the event log from its cursor). pb-web-server nests the router.
- `pb-leaderboard` (L5): the separate program; a section in the README says how to run it next to the bot or
  elsewhere.

## Storage
- Tokens and webhooks are events (`api.token.created`/`revoked`, `webhook.saved`/`removed`, audited; only token hashes
  and webhook secrets' existence are logged — the webhook secret itself lives in `secrets.toml`). Webhook cursors in a
  small file next to the settings (written after each delivery).

## Order
1. `pb-api-proto`, `pb-api` with tokens (owner UI comes with the redesigned System page), the read endpoints, the
   OpenAPI document; tests against the test web server.
2. Webhooks: dispatcher, signatures, cursors, retries; tests with a local receiver.
3. `pb-leaderboard` with a test that runs it against the test bot.
4. The UI: API tokens and webhooks under System, the API docs page.

New dependency: `utoipa` 6 (OpenAPI document from the types; pure Rust, maintained by its author since 2021). `hmac`
and `sha2` are already used.
