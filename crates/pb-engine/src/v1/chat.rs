//! Chat moderation (proposal 0008): a message in a community's text channel with a word or phrase of its word list
//! counts like a flagged sentence in a call. The moderation actor decides it with the same strikes and escalation
//! counts; what follows (deleting the message, the reply with the warning, the step's action, the reports) runs in the
//! enforcer like a sentence's.

use std::sync::Arc;

use jiff::Timestamp;
use pb_domain::{ChannelId, GuildId, Label, MessageId, UserId};
use pb_fluxer_api::{Destination, IncomingMessage, MessageRef, OutgoingMessage};
use pb_settings::{ChatWho, EscalationStep};
use pb_store_api::{ChatDeleted, ChatRecord, Event, MessagePurpose};
use pb_voicelines::{Line, Sel};
use pb_wordlist::WordList;

use super::core::Core;

/// A chat message with listed words, on its way to the moderation actor.
#[derive(Debug)]
pub struct ChatHeard {
    pub guild: GuildId,
    pub channel: ChannelId,
    pub message: MessageId,
    pub user: UserId,
    pub at: Timestamp,
    pub text: String,
    /// The entries of the word list that were found.
    pub matches: Vec<String>,
}

/// The message, when chat moderation reads it and it has a listed word or phrase.
pub fn consider(core: &Core, m: &IncomingMessage) -> Option<ChatHeard> {
    let g = m.guild?;
    let u = m.author.id;
    if m.author.bot || m.webhook || Some(u) == core.bot() || m.content.trim().is_empty() {
        return None;
    }
    let tree = core.settings.current();
    if !tree.guild_allowed(g) {
        return None;
    }
    let eff = tree.effective(Some(g), Some(u));
    if !eff.chat_moderation.value || eff.paused.value || eff.word_list.value.is_empty() {
        return None;
    }
    let channels = &eff.chat_channels.value;
    if !channels.is_empty() && !channels.contains(&m.channel) {
        return None;
    }
    // A command is not chat.
    let global = tree.effective(None, None);
    if global.commands_enabled.value
        && pb_commands::strip_prefix(&m.content, core.bot(), global.command_prefix.value.as_str()).is_some()
    {
        return None;
    }
    let counts = match eff.chat_who.value {
        ChatWho::Tracked => tree.is_tracked(g, u),
        ChatWho::Everyone => {
            pb_commands::level(super::commands::author_in(core, &tree, g, u, &m.author_roles))
                < pb_commands::Level::Admin
        }
    };
    if !counts {
        return None;
    }
    let entries: Vec<&str> = eff.word_list.value.iter().map(|w| w.as_str()).collect();
    let found = WordList::new(&entries).find(&m.content);
    if found.is_empty() {
        return None;
    }
    let mut matches: Vec<String> = found.into_iter().map(|f| f.pattern).collect();
    matches.dedup();
    Some(ChatHeard {
        guild: g,
        channel: m.channel,
        message: m.id,
        user: u,
        at: core.deps.clock.now(),
        text: m.content.clone(),
        matches,
    })
}

/// After the decision: the message deleted (when the settings say so), the warning as a reply, the step's action, and
/// the reports.
pub async fn follow_up(core: &Arc<Core>, record: ChatRecord, step: Option<EscalationStep>) {
    let (g, u) = (record.guild, record.user);
    let eff = core.settings.current().effective(Some(g), Some(u));
    let observe = eff.observe_only.value;
    let the_message = MessageRef {
        channel: record.channel,
        message: record.message,
    };
    if eff.chat_delete.value && !observe && record.decision.is_violation() {
        let reason = pb_i18n::text(super::reports::locale(core, Some(g)), "chat-delete-reason", &[]);
        let result = match core.ctl() {
            Some(ctl) => ctl
                .delete_message(the_message, Some(&reason))
                .await
                .map_err(|e| e.to_string()),
            None => Err("not connected to Fluxer".to_owned()),
        };
        if let Err(e) = &result {
            tracing::warn!(guild = %g, error = %e, "a flagged chat message could not be deleted");
        }
        core.record(vec![Event::ChatDeleted(ChatDeleted {
            id: record.id,
            ok: result.is_ok(),
            error: result.err(),
        })]);
    }
    if eff.chat_reply.value
        && let pb_store_api::DecisionRecord::Warn {
            step: step_no, count, ..
        } = record.decision
    {
        reply(core, &record, step_no, count, eff.chat_delete.value).await;
    }
    let action = match step.as_ref().and_then(|st| st.action.kind().map(|k| (st, k))) {
        Some((st, kind)) => {
            // Announced in the call they are in, if the bot is there too.
            let room = core.voice().of_user(u).find(|v| v.guild == g).and_then(|v| {
                core.rooms()
                    .into_iter()
                    .find(|r| r.chan.guild == g && r.chan.channel == v.channel)
            });
            Some(super::actions::step_action(core, (&record).into(), kind, st, room, None).await)
        }
        None => None,
    };
    super::reports::chat_flagged(core, &record, step.as_ref(), action.as_ref()).await;
}

/// The warning, written as a reply to the message (or after it, when the message was deleted).
async fn reply(core: &Arc<Core>, record: &ChatRecord, step: u32, count: u32, deleted: bool) {
    let (g, u) = (record.guild, record.user);
    let eff = core.settings.current().effective(Some(g), Some(u));
    let line = Line::Warning {
        label: Sel::Is(Label::Profanity),
        step: Sel::Is(step),
    };
    let fields = super::speak::warning_fields(&eff, step, count);
    let said = super::speak::line_text(core, g, u, Some(record.channel), &line, Some(Label::Profanity), &fields)
        .map(|(_, t)| t)
        .unwrap_or_else(|| pb_i18n::text(super::reports::locale(core, Some(g)), "chat-warning", &[]));
    let content = pb_i18n::text(
        super::reports::locale(core, Some(g)),
        "no-speak",
        &[("user", u.mention().into()), ("text", said.into())],
    );
    super::reports::send_with(
        core,
        Destination::Channel(record.channel),
        OutgoingMessage {
            content,
            reply_to: (!deleted).then_some(record.message),
            ping: vec![u],
            files: vec![],
        },
        MessagePurpose::ChatReply { record: record.id },
        Some(g),
    )
    .await;
}
