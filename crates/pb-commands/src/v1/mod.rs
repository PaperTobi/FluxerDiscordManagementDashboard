//! Version 1.

use pb_domain::{ChannelId, UserId};
use pb_settings::{FieldKind, SettingKey};

/// Who may run a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Anyone in the community.
    User,
    /// The community's owner, Administrator or Manage community, or an admin role.
    Admin,
    /// The bot owner and the extra bot admins, in every community.
    Operator,
}

/// What is known about a message's author, for [`level`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Author {
    /// The application owner or one of the extra bot admins.
    pub operator: bool,
    pub community_owner: bool,
    /// Holds one of the community's admin roles.
    pub admin_role: bool,
    /// Has Administrator or Manage community in the community.
    pub manages: bool,
    /// Is on the community's tracking list: never manages the bot there (tracking would mean nothing).
    pub tracked_here: bool,
}

/// The author's level (the same rule for chat commands and the web UI).
pub fn level(a: Author) -> Level {
    if a.operator {
        Level::Operator
    } else if !a.tracked_here && (a.community_owner || a.admin_role || a.manages) {
        Level::Admin
    } else {
        Level::User
    }
}

/// What `reset` resets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetWhat {
    All,
    Key(SettingKey),
}

/// A parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Status,
    List,
    Jar {
        user: Option<UserId>,
    },
    Add {
        users: Vec<UserId>,
    },
    Remove {
        users: Vec<UserId>,
    },
    Pause,
    Resume,
    Observe {
        on: bool,
    },
    /// `set <name> <value> [@user]`: the value as JSON, ready for the settings validator.
    Set {
        key: SettingKey,
        value: serde_json::Value,
        user: Option<UserId>,
    },
    Reset {
        what: ResetWhat,
        user: Option<UserId>,
    },
    /// `modlog #channel` or `modlog off`.
    Modlog {
        channel: Option<ChannelId>,
    },
}

impl Command {
    /// Who may run it (owner-only settings are checked again by the settings validator).
    pub fn required(&self) -> Level {
        match self {
            Command::Help | Command::Status | Command::List | Command::Jar { .. } => Level::User,
            _ => Level::Admin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command {name:?}")]
    Unknown { name: String },
    #[error("usage: {usage}")]
    Usage { usage: &'static str },
    #[error("name at least one user (a mention or a numeric id)")]
    NoUsers,
    #[error("expected at most one user after the value")]
    TooManyUsers,
    #[error("unknown setting {name:?}")]
    UnknownSetting { name: String },
    #[error("{name} is edited in the web UI")]
    WebOnly { name: String },
    #[error("name a channel like #mod-log")]
    NoChannel,
}

const ALIASES: &[(&str, &str)] = &[
    ("follow", "add"),
    ("track", "add"),
    ("watch", "add"),
    ("unfollow", "remove"),
    ("untrack", "remove"),
    ("unwatch", "remove"),
    ("rm", "remove"),
    ("ls", "list"),
    ("stop", "pause"),
    ("start", "resume"),
    ("observe-only", "observe"),
    ("?", "help"),
];

/// Short names for `set`/`reset` (any full setting key works too).
const SHORT_NAMES: &[(&str, &str)] = &[
    ("threshold", "threshold"),
    ("sensitivity", "threshold"),
    ("strikes", "strikes"),
    ("window", "strike_window"),
    ("strike-window", "strike_window"),
    ("audience", "audience"),
    ("language", "voice_language"),
    ("observe", "observe_only"),
];

/// The command text after the prefix or a mention of the bot, or `None` when the message is not a command.
pub fn strip_prefix<'a>(content: &'a str, bot: Option<UserId>, prefix: &str) -> Option<&'a str> {
    let text = content.trim();
    if let Some(bot) = bot {
        for mention in [format!("<@{bot}>"), format!("<@!{bot}>")] {
            if let Some(rest) = text.strip_prefix(mention.as_str()) {
                return Some(rest.trim());
            }
        }
    }
    if prefix.is_empty() {
        return None;
    }
    let head = text.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = &text[prefix.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

fn user(token: &str) -> Option<UserId> {
    UserId::from_mention(token)
}

fn channel(token: &str) -> Option<ChannelId> {
    ChannelId::from_mention(token)
}

fn users(tokens: &[&str]) -> Result<Vec<UserId>, CommandError> {
    let mut out: Vec<UserId> = Vec::new();
    for t in tokens {
        if let Some(u) = user(t)
            && !out.contains(&u)
        {
            out.push(u);
        }
    }
    if out.is_empty() {
        Err(CommandError::NoUsers)
    } else {
        Ok(out)
    }
}

fn optional_user(tokens: &[&str]) -> Result<Option<UserId>, CommandError> {
    match tokens {
        [] => Ok(None),
        [one] => user(one).map(Some).ok_or(CommandError::TooManyUsers),
        _ => Err(CommandError::TooManyUsers),
    }
}

fn setting_key(name: &str) -> Result<SettingKey, CommandError> {
    let lower = name.to_ascii_lowercase();
    let full = SHORT_NAMES
        .iter()
        .find(|(s, _)| *s == lower)
        .map_or(lower.as_str(), |(_, k)| k);
    let key: SettingKey = full
        .parse()
        .map_err(|_| CommandError::UnknownSetting { name: name.to_owned() })?;
    if matches!(key.meta().kind, FieldKind::Escalation | FieldKind::Voices) {
        return Err(CommandError::WebOnly { name: key.name() });
    }
    Ok(key)
}

/// Parses the text after the prefix.
pub fn parse(args: &str) -> Result<Command, CommandError> {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    let Some(first) = tokens.first() else {
        return Ok(Command::Help);
    };
    let lower = first.to_ascii_lowercase();
    let name = ALIASES
        .iter()
        .find(|(a, _)| *a == lower)
        .map_or(lower.as_str(), |(_, c)| c);
    let rest = &tokens[1..];
    Ok(match name {
        "help" => Command::Help,
        "status" => Command::Status,
        "list" => Command::List,
        "jar" => Command::Jar {
            user: rest.iter().find_map(|t| user(t)),
        },
        "add" => Command::Add { users: users(rest)? },
        "remove" => Command::Remove { users: users(rest)? },
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "observe" => Command::Observe {
            on: rest
                .first()
                .and_then(|w| pb_settings::switch_word(w))
                .ok_or(CommandError::Usage {
                    usage: "observe on|off",
                })?,
        },
        "set" => {
            if rest.len() < 2 {
                return Err(CommandError::Usage {
                    usage: "set <setting> <value> [@user]",
                });
            }
            let key = setting_key(rest[0])?;
            // A person is named by a mention at the end; every word before it is the value (lists, `1h 30m`).
            let (words, user) = match rest[1..].split_last() {
                Some((last, before)) if !before.is_empty() && last.starts_with("<@") && !last.starts_with("<@&") => {
                    (before, Some(user(last).ok_or(CommandError::TooManyUsers)?))
                }
                _ => (&rest[1..], None),
            };
            Command::Set {
                key,
                value: pb_settings::text_value(key, &words.join(" ")),
                user,
            }
        }
        "reset" => {
            let Some(what) = rest.first() else {
                return Err(CommandError::Usage {
                    usage: "reset <setting|all> [@user]",
                });
            };
            let what = if what.eq_ignore_ascii_case("all") {
                ResetWhat::All
            } else {
                ResetWhat::Key(setting_key(what)?)
            };
            Command::Reset {
                what,
                user: optional_user(&rest[1..])?,
            }
        }
        "modlog" => match rest.first() {
            None => {
                return Err(CommandError::Usage {
                    usage: "modlog #channel|off",
                });
            }
            Some(w) if matches!(w.to_ascii_lowercase().as_str(), "off" | "none" | "disable") => {
                Command::Modlog { channel: None }
            }
            Some(w) => Command::Modlog {
                channel: Some(channel(w).ok_or(CommandError::NoChannel)?),
            },
        },
        other => return Err(CommandError::Unknown { name: other.to_owned() }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_prefixes_and_mentions() {
        assert_eq!(strip_prefix("!pb add <@5>", None, "!pb"), Some("add <@5>"));
        assert_eq!(strip_prefix("!PB status", None, "!pb"), Some("status"));
        assert_eq!(strip_prefix("!pbx status", None, "!pb"), None);
        assert_eq!(strip_prefix("<@9> list", Some(UserId(9)), "!pb"), Some("list"));
        assert_eq!(strip_prefix("<@!9>", Some(UserId(9)), ""), Some(""));
        assert_eq!(strip_prefix("hello", Some(UserId(9)), ""), None);
    }

    #[test]
    fn parses_every_command() {
        assert_eq!(parse(""), Ok(Command::Help));
        assert_eq!(
            parse("track <@1> 2 <@!3> <@1>"),
            Ok(Command::Add {
                users: vec![UserId(1), UserId(2), UserId(3)]
            })
        );
        assert_eq!(parse("rm"), Err(CommandError::NoUsers));
        assert_eq!(parse("observe on"), Ok(Command::Observe { on: true }));
        assert_eq!(parse("observe aus"), Ok(Command::Observe { on: false }), "German words");
        assert_eq!(
            parse("set threshold 0,6 <@4>"),
            Ok(Command::Set {
                key: SettingKey::Threshold,
                value: serde_json::json!("0,6"),
                user: Some(UserId(4))
            })
        );
        assert_eq!(
            parse("set fallback_languages de en"),
            Ok(Command::Set {
                key: SettingKey::FallbackLanguages,
                value: serde_json::json!(["de", "en"]),
                user: None
            }),
            "every word is the value"
        );
        assert_eq!(
            parse("set admin_role_ids <@&1> <@&2>"),
            Ok(Command::Set {
                key: SettingKey::AdminRoleIds,
                value: serde_json::json!(["1", "2"]),
                user: None
            }),
            "role mentions are values, not people"
        );
        assert_eq!(
            parse("set window unlimited"),
            Ok(Command::Set {
                key: SettingKey::StrikeWindow,
                value: serde_json::json!("unlimited"),
                user: None
            })
        );
        assert_eq!(
            parse("set strikes 3"),
            Ok(Command::Set {
                key: SettingKey::Strikes,
                value: serde_json::json!("3"),
                user: None
            })
        );
        assert_eq!(
            parse("set greet_enabled on"),
            Ok(Command::Set {
                key: SettingKey::GreetEnabled,
                value: serde_json::json!("on"),
                user: None
            })
        );
        assert_eq!(
            parse("set language auto <@4>"),
            Ok(Command::Set {
                key: SettingKey::VoiceLanguage,
                value: serde_json::json!("auto"),
                user: Some(UserId(4))
            })
        );
        assert!(matches!(parse("set escalation x"), Err(CommandError::WebOnly { .. })));
        assert_eq!(
            parse("reset all <@4>"),
            Ok(Command::Reset {
                what: ResetWhat::All,
                user: Some(UserId(4))
            })
        );
        assert_eq!(
            parse("modlog <#77>"),
            Ok(Command::Modlog {
                channel: Some(ChannelId(77))
            })
        );
        assert_eq!(parse("modlog off"), Ok(Command::Modlog { channel: None }));
        assert!(matches!(parse("dance"), Err(CommandError::Unknown { .. })));
        assert_eq!(parse("add <@1>").map(|c| c.required()), Ok(Level::Admin));
        assert_eq!(parse("jar").map(|c| c.required()), Ok(Level::User));
    }

    #[test]
    fn levels() {
        assert_eq!(level(Author::default()), Level::User);
        assert_eq!(
            level(Author {
                manages: true,
                ..Author::default()
            }),
            Level::Admin
        );
        assert_eq!(
            level(Author {
                operator: true,
                ..Author::default()
            }),
            Level::Operator
        );
        assert_eq!(
            level(Author {
                community_owner: true,
                tracked_here: true,
                ..Author::default()
            }),
            Level::User,
            "tracked people never manage the bot where they are tracked"
        );
        assert!(Level::Operator > Level::Admin && Level::Admin > Level::User);
    }
}
