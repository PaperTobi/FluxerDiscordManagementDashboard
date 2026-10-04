//! The chat commands as a reference in the web UI. Fluxer has no slash commands and no autocomplete: people type a
//! command as an ordinary message (the prefix or a mention of the bot, then the command). This shows the current
//! prefix and every command with what it does and who may use it, from the bot's own `help` answer (the one list of
//! commands), and links settings that have a command of their own to it.

use leptos::prelude::*;
use pb_domain::{GuildId, Scope};
use pb_i18n::{Locale, text};
use pb_settings::SettingKey;

use crate::app::app;

/// Where the reference is: on the System page (in its chat commands settings) and on each community's overview.
pub const ANCHOR: &str = "chat-commands";

/// Settings with chat commands of their own (written after the prefix).
const COMMANDS: &[(SettingKey, &[&str])] = &[
    (SettingKey::Paused, &["pause", "resume"]),
    (SettingKey::ObserveOnly, &["observe on|off"]),
    (SettingKey::Threshold, &["set threshold 0.6 [@user]"]),
    (SettingKey::Strikes, &["set strikes 2 [@user]"]),
    (SettingKey::StrikeWindow, &["set window 20s [@user]"]),
    (SettingKey::Audience, &["set audience offender|tracked|channel [@user]"]),
    (SettingKey::VoiceLanguage, &["set language de [@user]"]),
    (SettingKey::ModlogChannel, &["modlog #channel", "modlog off"]),
    (SettingKey::JarEnabled, &["jar [@user]"]),
];

/// The bot's command prefix.
fn prefix() -> String {
    app()
        .engine
        .settings()
        .current()
        .effective(None, None)
        .command_prefix
        .value
        .to_string()
}

/// For a setting row in a community or for a person: its chat commands, linked to the reference on the community's
/// overview (`None` when it has none there; at the global scope chat commands do not act, and for a person only those
/// that name one do).
pub fn setting_commands(key: SettingKey, scope: Scope, loc: Locale) -> Option<AnyView> {
    let guild = scope.guild()?;
    let person = matches!(scope, Scope::Person { .. });
    let commands: Vec<&str> = COMMANDS
        .iter()
        .find(|(k, _)| *k == key)?
        .1
        .iter()
        .copied()
        .filter(|c| !person || c.contains("@user"))
        .collect();
    if commands.is_empty() {
        return None;
    }
    let prefix = prefix();
    Some(
        view! {
            <a class="chat-command small" href=format!("/c/{guild}#{ANCHOR}") title=text(loc, "ui-chat-commands", &[])>
                {commands.iter().map(|c| view! { <code>{format!("{prefix} {c}")}</code> }).collect_view()}
            </a>
        }
        .into_any(),
    )
}

/// One line of chat markup as HTML: `code` and **bold**, the rest as text.
fn markup(line: &str) -> AnyView {
    line.split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                view! { <code>{part.to_owned()}</code> }.into_any()
            } else {
                part.split("**")
                    .enumerate()
                    .map(|(j, t)| {
                        if j % 2 == 1 {
                            view! { <b>{t.to_owned()}</b> }.into_any()
                        } else {
                            t.to_owned().into_any()
                        }
                    })
                    .collect_view()
                    .into_any()
            }
        })
        .collect_view()
        .into_any()
}

/// The chat commands, folded: how to write one, the prefix, each command with what it does and who may use it.
/// `guild`: the community it is shown for (its mod-log audio setting changes one line).
#[component]
pub fn ChatCommands(guild: Option<GuildId>, locale: Locale) -> impl IntoView {
    let tree = app().engine.settings().current();
    let on = tree.effective(None, None).commands_enabled.value;
    let audio = tree.effective(guild, None).modlog_audio.value;
    let prefix = prefix();
    let help = text(
        locale,
        "cmd-help",
        &[("prefix", prefix.clone().into()), ("audio", audio.into())],
    );
    view! {
        <details class="chat-commands" id=ANCHOR>
            <summary>{text(locale, "ui-chat-commands", &[])}</summary>
            <p class="muted small">{text(locale, "ui-chat-commands-how", &[("prefix", prefix.into())])}</p>
            {(!on).then(|| view! { <p class="notice warn">{text(locale, "ui-chat-commands-off", &[])}</p> })}
            <div class="chat-help">
                {help.lines().map(|l| view! { <p>{markup(l.trim())}</p> }).collect_view()}
            </div>
        </details>
    }
}
