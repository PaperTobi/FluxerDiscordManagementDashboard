//! Chat commands (`!pb …` or a mention of the bot), answered in the community's chat language, and the bot's custom
//! status text.

use std::sync::Arc;

use pb_commands::{Author, Command, CommandError, Level, ResetWhat, level, parse, strip_prefix};
use pb_domain::{GuildId, RoleId, Scope, UserId};
use pb_fluxer_api::{Destination, IncomingMessage, MessageRef, OutgoingMessage, perms};
use pb_i18n::{Arg, Locale, duration, setting_name, text};
use pb_settings::{SettingError, SettingKey, SettingsTree};
use pb_store_api::{Actor, Event, JarReset, Via};

use super::core::Core;
use super::moderation::ModMsg;

/// The bot's custom status.
pub fn presence_text(core: &Core) -> String {
    let tree = core.settings.current();
    let loc = Locale::for_lang(&tree.effective(None, None).chat_language.value);
    let allow = tree.effective(None, None).guild_allowlist.value.clone();
    let guilds: Vec<GuildId> = if allow.is_empty() {
        core.guilds().available().into_iter().collect()
    } else {
        allow
    };
    if !guilds.is_empty() && guilds.iter().all(|g| tree.effective(Some(*g), None).paused.value) {
        return text(loc, "presence-paused", &[]);
    }
    let people: std::collections::BTreeSet<UserId> = guilds
        .iter()
        .flat_map(|g| tree.tracked_for(*g).iter().copied().collect::<Vec<_>>())
        .collect();
    if people.is_empty() {
        return text(loc, "presence-nobody", &[]);
    }
    let observe = !guilds.is_empty() && guilds.iter().all(|g| tree.effective(Some(*g), None).observe_only.value);
    text(
        loc,
        "presence-watching",
        &[("count", people.len().into()), ("observe", observe.into())],
    )
}

fn mentions(us: &[UserId]) -> String {
    us.iter().map(|u| u.mention()).collect::<Vec<_>>().join(", ")
}

/// A settings edit made by a command.
type Edit = Box<dyn FnOnce(&mut pb_settings::SettingsTree) -> Result<Vec<pb_settings::Change>, SettingError> + Send>;

/// A command's answer and the reaction on the command message.
struct Answer {
    text: String,
    emoji: Option<&'static str>,
}

fn ok(text: String) -> Answer {
    Answer {
        text,
        emoji: Some("✅"),
    }
}

fn info(text: String) -> Answer {
    Answer { text, emoji: None }
}

fn fail(text: String) -> Answer {
    Answer {
        text,
        emoji: Some("❌"),
    }
}

fn usage_error(loc: Locale, prefix: &str, e: &CommandError) -> String {
    match e {
        CommandError::Unknown { name } => text(loc, "cmd-unknown", &[("name", name.into()), ("prefix", prefix.into())]),
        CommandError::Usage { usage } => text(
            loc,
            "cmd-usage",
            &[("prefix", prefix.into()), ("usage", (*usage).into())],
        ),
        CommandError::NoUsers => text(loc, "cmd-no-users", &[]),
        CommandError::TooManyUsers => text(loc, "cmd-too-many-users", &[]),
        CommandError::UnknownSetting { name } => text(loc, "cmd-unknown-setting", &[("name", name.into())]),
        CommandError::WebOnly { name } => {
            let shown = name
                .parse::<SettingKey>()
                .map_or_else(|_| name.clone(), |k| setting_name(loc, k));
            text(loc, "cmd-web-only", &[("setting", shown.into())])
        }
        CommandError::NoChannel => text(loc, "cmd-no-channel", &[]),
    }
}

/// Why a setting was refused, for the chat (an unknown name gets examples of known ones).
fn setting_error(loc: Locale, e: &SettingError) -> String {
    match e {
        SettingError::Unknown(k) => text(loc, "cmd-unknown-setting", &[("name", k.into())]),
        other => pb_i18n::setting_error(loc, other),
    }
}

/// What decides someone's command level in a community (their roles there given).
pub(super) fn author_in(core: &Core, tree: &SettingsTree, g: GuildId, user: UserId, roles: &[RoleId]) -> Author {
    let gs = core.guilds();
    Author {
        operator: core.owner() == Some(user) || tree.effective(None, None).admin_user_ids.value.contains(&user),
        community_owner: gs.get(g).and_then(|i| i.owner) == Some(user),
        admin_role: tree
            .effective(Some(g), None)
            .admin_role_ids
            .value
            .iter()
            .any(|r| roles.contains(r)),
        manages: gs.permissions(g, user, roles, None) & (perms::ADMINISTRATOR | perms::MANAGE_GUILD) != 0,
        tracked_here: tree.listed_for(g).contains(&user),
    }
}

/// Handles a message that may be a command.
pub async fn on_message(core: &Arc<Core>, m: &IncomingMessage) {
    let tree = core.settings.current();
    let global = tree.effective(None, None);
    if !global.commands_enabled.value || m.author.bot || m.webhook {
        return;
    }
    let bot = core.bot();
    if Some(m.author.id) == bot {
        return;
    }
    let prefix = global.command_prefix.value.as_str().to_owned();
    let Some(args) = strip_prefix(&m.content, bot, &prefix) else {
        return;
    };
    let Some(ctl) = core.ctl() else { return };
    let loc = Locale::for_lang(&tree.effective(m.guild, None).chat_language.value);
    let shown_prefix = if prefix.is_empty() {
        bot.map_or_else(|| "@bot".to_owned(), UserId::mention)
    } else {
        prefix.clone()
    };
    // Communities outside the allowlist get no answers at all.
    if m.guild.is_some_and(|g| !tree.guild_allowed(g)) {
        return;
    }
    let answer = match m.guild {
        None => info(text(loc, "cmd-dm-only", &[])),
        Some(g) => match parse(args) {
            Err(e) => fail(usage_error(loc, &shown_prefix, &e)),
            Ok(cmd) => run(core, m, g, loc, &shown_prefix, cmd).await,
        },
    };
    if let Some(e) = answer.emoji
        && let Err(err) = ctl
            .react(
                MessageRef {
                    channel: m.channel,
                    message: m.id,
                },
                e,
            )
            .await
    {
        tracing::debug!(error = %err, "a command reaction failed");
    }
    if !answer.text.is_empty() {
        let reply = OutgoingMessage {
            content: answer.text,
            reply_to: Some(m.id),
            ping: vec![],
            files: vec![],
        };
        if let Err(e) = ctl.send(Destination::Channel(m.channel), reply).await {
            tracing::warn!(error = %e, "could not answer a command");
        }
    }
}

async fn run(core: &Arc<Core>, m: &IncomingMessage, g: GuildId, loc: Locale, prefix: &str, cmd: Command) -> Answer {
    let tree = core.settings.current();
    let global = tree.effective(None, None);
    let author = m.author.id;
    let known = core.guilds().get(g).is_some_and(|i| !i.roles.is_empty());
    let who = author_in(core, &tree, g, author, &m.author_roles);
    let lvl = level(who);
    if cmd.required() > Level::User {
        if lvl < Level::Operator && !known {
            return info(text(loc, "cmd-roles-loading", &[]));
        }
        if lvl < cmd.required() {
            return Answer {
                text: text(loc, "cmd-denied", &[]),
                emoji: Some("🚫"),
            };
        }
    }
    let actor = Actor {
        user: Some(author),
        name: Some(core.guilds().name(g, author)),
        via: Via::Chat,
    };
    let by_owner = lvl == Level::Operator;
    let server = Scope::Server { guild: g };
    let scope_of = |user: Option<UserId>| user.map_or(server, |u| Scope::Person { guild: g, user: u });
    let change = |f: Edit| {
        let actor = actor.clone();
        async move { core.settings.change(actor, f).await }
    };
    let saved = |r: Result<Vec<pb_settings::Change>, super::settings::ChangeError>, done: String| match r {
        Ok(_) => ok(done),
        Err(super::settings::ChangeError::Setting(e)) => fail(setting_error(loc, &e)),
        Err(super::settings::ChangeError::Store(e)) => {
            fail(text(loc, "cmd-store-failed", &[("error", e.to_string().into())]))
        }
    };
    match cmd {
        Command::Help => {
            let audio = tree.effective(Some(g), None).modlog_audio.value;
            let mut t = text(loc, "cmd-help", &[("prefix", prefix.into()), ("audio", audio.into())]);
            t.push('\n');
            t.push_str(&match global.ui_url.value.as_ref() {
                Some(url) => text(loc, "cmd-help-ui", &[("url", url.to_string().into())]),
                None => text(loc, "cmd-help-no-ui", &[]),
            });
            info(t)
        }
        Command::Status => info(status(core, g, loc)),
        Command::List => {
            let listed = tree.listed_for(g);
            if listed.is_empty() {
                return info(text(loc, "cmd-list-empty", &[("prefix", prefix.into())]));
            }
            let everywhere = global.tracked_everywhere.value.clone();
            let mut lines = vec![text(
                loc,
                "cmd-list-head",
                &[
                    ("count", listed.len().into()),
                    ("paused", tree.effective(Some(g), None).paused.value.into()),
                ],
            )];
            for u in listed.iter() {
                let mut l = if everywhere.contains(u) {
                    text(loc, "cmd-list-everywhere", &[("user", u.mention().into())])
                } else {
                    text(loc, "cmd-list-person", &[("user", u.mention().into())])
                };
                if let Some(t) = tree
                    .overrides(Scope::Person { guild: g, user: *u })
                    .get_json(SettingKey::Threshold)
                    .and_then(|v| v.as_f64())
                {
                    l.push_str(&text(
                        loc,
                        "cmd-list-note-threshold",
                        &[("threshold", loc.fixed(t, 2).into())],
                    ));
                }
                lines.push(l);
            }
            info(lines.join("\n"))
        }
        Command::Jar { user } => {
            if !tree.effective(Some(g), user).jar_enabled.value {
                return info(text(loc, "cmd-jar-off", &[]));
            }
            if let Some(u) = user {
                return info(text(
                    loc,
                    "cmd-jar-person",
                    &[("user", u.mention().into()), ("count", core.jar(g, u).into())],
                ));
            }
            let rows = core.deps.index.jar(Some(g)).await.unwrap_or_default();
            if rows.is_empty() {
                return info(text(loc, "cmd-jar-empty", &[]));
            }
            let mut lines = vec![text(loc, "cmd-jar-head", &[])];
            for (i, r) in rows.iter().enumerate() {
                lines.push(text(
                    loc,
                    "cmd-jar-line",
                    &[
                        ("rank", (i + 1).into()),
                        ("user", r.user.mention().into()),
                        ("count", r.count.into()),
                    ],
                ));
            }
            info(lines.join("\n"))
        }
        Command::Add { users } => match super::people::track(core, g, &users, actor.clone()).await {
            Err(super::people::TrackError::TheBot) => fail(text(loc, "cmd-add-self", &[])),
            Err(super::people::TrackError::Change(e)) => saved(Err(e), String::new()),
            Ok(t) => {
                let mut out = Vec::new();
                if !t.added.is_empty() {
                    out.push(text(loc, "cmd-add-done", &[("people", mentions(&t.added).into())]));
                }
                if !t.already.is_empty() {
                    out.push(text(loc, "cmd-add-already", &[("people", mentions(&t.already).into())]));
                }
                ok(out.join(" "))
            }
        },
        Command::Remove { users } => match super::people::untrack(core, g, &users, actor.clone()).await {
            Err(e) => saved(Err(e), String::new()),
            Ok(u) => {
                let mut out = Vec::new();
                if !u.removed.is_empty() {
                    out.push(text(loc, "cmd-remove-done", &[("people", mentions(&u.removed).into())]));
                }
                if !u.everywhere.is_empty() {
                    out.push(text(
                        loc,
                        "cmd-remove-everywhere",
                        &[
                            ("people", mentions(&u.everywhere).into()),
                            ("count", u.everywhere.len().into()),
                        ],
                    ));
                }
                if !u.missing.is_empty() {
                    out.push(text(
                        loc,
                        "cmd-remove-missing",
                        &[("people", mentions(&u.missing).into())],
                    ));
                }
                ok(out.join(" "))
            }
        },
        Command::Pause | Command::Resume => {
            let on = matches!(cmd, Command::Pause);
            if !on {
                core.resume_joining(g);
            }
            let r = change(Box::new(move |t| {
                Ok(t.set(server, SettingKey::Paused, serde_json::json!(on), by_owner)?
                    .into_iter()
                    .collect())
            }))
            .await;
            saved(
                r,
                if on {
                    text(loc, "cmd-pause", &[("prefix", prefix.into())])
                } else {
                    text(loc, "cmd-resume", &[])
                },
            )
        }
        Command::Observe { on } => {
            let r = change(Box::new(move |t| {
                Ok(t.set(server, SettingKey::ObserveOnly, serde_json::json!(on), by_owner)?
                    .into_iter()
                    .collect())
            }))
            .await;
            saved(r, text(loc, if on { "cmd-observe-on" } else { "cmd-observe-off" }, &[]))
        }
        Command::Set { key, value, user } => {
            let scope = scope_of(user);
            let r = change(Box::new(move |t| {
                Ok(t.set(scope, key, value, by_owner)?.into_iter().collect())
            }))
            .await;
            let after = core
                .settings
                .current()
                .overrides(scope)
                .get_json(key)
                .unwrap_or(serde_json::Value::Null);
            let name = setting_name(loc, key);
            let done = match user {
                Some(u) => text(
                    loc,
                    "cmd-set-person",
                    &[
                        ("setting", name.into()),
                        ("value", pb_i18n::setting_value(loc, key, &after).into()),
                        ("user", u.mention().into()),
                    ],
                ),
                None => text(
                    loc,
                    "cmd-set-community",
                    &[
                        ("setting", name.into()),
                        ("value", pb_i18n::setting_value(loc, key, &after).into()),
                    ],
                ),
            };
            saved(r, done)
        }
        Command::Reset { what, user } => {
            let scope = scope_of(user);
            let done = match (&what, user) {
                (ResetWhat::All, Some(u)) => text(loc, "cmd-reset-all-person", &[("user", u.mention().into())]),
                (ResetWhat::All, None) => text(loc, "cmd-reset-all-community", &[]),
                (ResetWhat::Key(k), Some(u)) => text(
                    loc,
                    "cmd-reset-person",
                    &[("setting", setting_name(loc, *k).into()), ("user", u.mention().into())],
                ),
                (ResetWhat::Key(k), None) => {
                    text(loc, "cmd-reset-community", &[("setting", setting_name(loc, *k).into())])
                }
            };
            // The mod-log channel stays: it is switched off on its own (`modlog off`).
            let r = change(Box::new(move |t| {
                Ok(match what {
                    ResetWhat::All => t.reset(scope, &[SettingKey::ModlogChannel], by_owner),
                    ResetWhat::Key(k) => t.clear(scope, k, by_owner)?.into_iter().collect(),
                })
            }))
            .await;
            saved(r, done)
        }
        Command::Modlog { channel } => {
            let Some(c) = channel else {
                let r = change(Box::new(move |t| {
                    Ok(t.clear(server, SettingKey::ModlogChannel, by_owner)?
                        .into_iter()
                        .collect())
                }))
                .await;
                return saved(r, text(loc, "cmd-modlog-off", &[]));
            };
            let kind = core.guilds().get(g).and_then(|i| i.channels.get(&c).map(|ch| ch.kind));
            match kind {
                None => return fail(text(loc, "cmd-channel-unknown", &[])),
                Some(k) if !k.holds_messages() => return fail(text(loc, "cmd-channel-not-text", &[])),
                Some(_) => {}
            }
            let r = change(Box::new(move |t| {
                Ok(t.set(
                    server,
                    SettingKey::ModlogChannel,
                    serde_json::json!(c.to_string()),
                    by_owner,
                )?
                .into_iter()
                .collect())
            }))
            .await;
            let audio = tree.effective(Some(g), None).modlog_audio.value;
            let mut done = text(
                loc,
                "cmd-modlog-set",
                &[("channel", c.mention().into()), ("audio", audio.into())],
            );
            let need = perms::VIEW_CHANNEL | perms::SEND_MESSAGES | if audio { perms::ATTACH_FILES } else { 0 };
            let missing = perms::missing(core.guilds().bot_permissions(g, Some(c)), need);
            if !missing.is_empty() {
                done.push('\n');
                done.push_str(&text(
                    loc,
                    "cmd-missing-permissions",
                    &[("permissions", permission_names(loc, &missing).into())],
                ));
            }
            saved(r, done)
        }
    }
}

/// Permission names as Fluxer shows them, joined.
fn permission_names(loc: Locale, names: &[&str]) -> String {
    let names: Vec<String> = names.iter().map(|n| pb_i18n::permission_name(loc, n)).collect();
    names.join(", ")
}

fn status(core: &Core, g: GuildId, loc: Locale) -> String {
    let tree = core.settings.current();
    let eff = tree.effective(Some(g), None);
    let tracked: Vec<UserId> = tree.tracked_for(g).iter().copied().collect();
    let mut lines = vec![text(loc, "cmd-status-head", &[("guild", g.to_string().into())])];
    if eff.paused.value {
        lines.push(text(loc, "cmd-status-paused", &[]));
    } else {
        let people = if tracked.is_empty() {
            "none".to_owned()
        } else {
            mentions(&tracked)
        };
        lines.push(text(
            loc,
            "cmd-status-following",
            &[("count", tracked.len().into()), ("people", people.into())],
        ));
    }
    lines.push(text(
        loc,
        if eff.observe_only.value {
            "cmd-status-mode-observe"
        } else {
            "cmd-status-mode-warn"
        },
        &[],
    ));
    lines.push(text(
        loc,
        "cmd-status-detection",
        &[
            ("threshold", loc.fixed(eff.threshold.value.get(), 2).into()),
            ("strikes", eff.strikes.value.get().into()),
            (
                "window",
                duration(loc, eff.strike_window.value.value().map(|d| d.get().get())).into(),
            ),
            ("audience", eff.audience.value.as_str().into()),
        ],
    ));
    lines.push(match eff.modlog_channel.value {
        Some(c) => text(
            loc,
            "cmd-status-modlog",
            &[
                ("channel", c.mention().into()),
                ("audio", eff.modlog_audio.value.into()),
            ],
        ),
        None => text(loc, "cmd-status-modlog-off", &[]),
    });
    for r in core.rooms().iter().filter(|r| r.chan.guild == g) {
        let listening: Vec<UserId> = core
            .voice()
            .in_channel(g, r.chan.channel)
            .map(|v| v.user)
            .filter(|u| tracked.contains(u))
            .collect();
        let people = if listening.is_empty() {
            "none".to_owned()
        } else {
            mentions(&listening)
        };
        lines.push(text(
            loc,
            "cmd-status-room",
            &[("channel", r.chan.channel.mention().into()), ("people", people.into())],
        ));
    }
    let info = core.deps.inference.classifier_info();
    lines.push(text(
        loc,
        "cmd-status-model",
        &[
            ("ready", core.deps.inference.stuck().is_none().into()),
            ("device", Arg::from(info.device.clone())),
        ],
    ));
    lines.join("\n")
}

/// Empties a person's swear jar (web UI).
pub async fn reset_jar(core: &Core, g: GuildId, u: UserId, by: Actor) {
    core.record(vec![Event::JarReset(JarReset { guild: g, user: u, by })]);
    let _ = core.moderation.send(ModMsg::JarReset(g, u));
}

#[cfg(test)]
mod tests {
    use pb_fluxer_api::perms;
    use pb_i18n::{Catalog, Locale};

    #[test]
    fn every_permission_the_bot_names_has_a_name_in_every_locale() {
        for locale in Locale::ALL {
            for (_, name) in perms::NAMES {
                assert!(
                    Catalog::get().has(locale, &format!("perm-{name}")),
                    "{locale:?} perm-{name}"
                );
            }
        }
    }
}
