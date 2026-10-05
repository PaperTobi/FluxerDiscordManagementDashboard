# 0008 — Chat moderation: word lists in text channels, the same strikes and escalation as in calls

Date: 2026-10-05. Status: steps 1–3 built (the user chose "text chat moderation" in the quiz of 2026-10-04); step 4
(the web UI's lists of flagged messages) comes with the redesigned pages.

## What it does
A message in a community's text channel that contains a word or phrase from the community's word list is a flagged
message. It counts like a flagged sentence in a call: strikes first (if set), then a violation that moves the person
up the escalation steps (warning, mute, time-out, …) **shared with their calls** — three violations in chat and two in
a call are five. The bot can delete the message and answer in the channel with the warning (the warning line's text,
in the person's language). Reports go to the mod log and the owner like those of calls, with the message instead of a
recording. Only flagged messages are kept; other chat is read and forgotten.

The same word list later counts in transcripts of what is said in calls (proposal 0007, speech recognition).

## Settings (a new section "Chat", every scope)
| key | type | default | meaning |
|---|---|---|---|
| `chat_moderation` | switch | off | read the community's chat |
| `chat_who` | choice `tracked` / `everyone` | `tracked` | whose messages count (tracked people, like calls; or everyone but admins) |
| `word_list` | list of words and phrases | empty | what counts; `*` at a word's start or end matches any letters there (`fuck*`) |
| `chat_channels` | list of channels | empty = every text channel | where |
| `chat_delete` | switch | off | delete a flagged message (needs Manage Messages; the bot says when it lacks it) |
| `chat_reply` | switch | on | answer in the channel with the warning |

The list is one setting like every other (the most specific scope wins). The web form edits it one entry at a time;
chat commands add and remove words.

## Matching (a new pure crate `pb-wordlist`)
- Text and patterns are normalised the same way: Unicode NFKC and lower case, zero-width characters removed, common
  look-alikes folded (`0→o 1→i 3→e 4→a 5→s 7→t @→a $→s`), letters repeated three or more times folded to two.
- A pattern matches whole words (`ass` does not match `class`); `*` makes one end open; a phrase matches its words in
  order with only spaces or punctuation between them.
- `WordList::compile(&[String]) -> WordList` once per settings change, `find(&self, text) -> Vec<Match {pattern,
  range}>`. Property tests: normalising twice is normalising once; a pattern always matches itself.
- Dependency: `unicode-normalization` (pure Rust, maintained by the unicode-rs project) — vetted with
  `cargo xtask freshness` before it is added.

## Engine
- `GatewayEvent::Message` already reaches the engine (chat commands). A message that is not a command, from a person
  whose messages count, in a channel that counts, goes to the moderation actor as `ModMsg::Chat`.
- The moderation actor decides it with the same `Decider` (strikes) and `Violations` (escalation counts) as sentences:
  one history per person, so both kinds count together. `pb-policy` gains nothing; the input is the same.
- `Followup` (the enforcer's work) carries `Evidence::Sentence { sentence, wav } | Evidence::Message(record)`; the
  action is the same, reports word the evidence.
- Deleting and replying go through `FluxerCtl` (delete message, send message with a reply reference); both are
  recorded (`message.sent` already is; a deletion is part of the chat record).

## Store
- Event `chat.flagged` v1: `ChatRecord { id, guild, channel, message, user, at, text, matches, decision, jar }` (the
  decision as for sentences; a match counts as profanity at score 1); `chat.deleted` v1 notes the deletion, and the
  reply is a `message.sent` with the purpose `chat_reply`. Indexed in a `messages` table; `violation_times` reads both tables, so
  the counts seeded at start include chat.
- The swear jar counts chat violations too (it counts violations, wherever they happen).

## Web
- The person page and the community's live view list flagged messages beside sentences (a chat bubble instead of a
  player). The wall shows them.
- Settings: the Chat section; the word list as a list editor with a "try it" box that shows what a text would match.

## Order
1. `pb-wordlist` with its tests.
2. Settings, the `chat.flagged` event and index, `violation_times` from both.
3. The engine path (decide, record, act, delete, reply) with scenario tests on the fake Fluxer (it needs message
   deletion and replies).
4. Reports and the web UI.
