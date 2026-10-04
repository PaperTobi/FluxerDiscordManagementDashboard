//! Old settings rows → the settings tree (and so the TOML files).

use std::collections::BTreeMap;

use jiff::Timestamp;
use pb_domain::{BlobHash, GuildId, Label, Lang, Scope, UserId};
use pb_settings::{Change, SettingKey, SettingsTree};
use pb_voicelines::{Line, LineKey, Sel, Slot};
use serde_json::{Value, json};

use super::old::OldDb;

/// The old bot's default text-to-speech voice (its language is the language of texts at scopes without a voice).
const OLD_DEFAULT_VOICE: &str = "en_US-lessac-medium";
/// The old bot's "every community" (tracked rows and person settings `*:<user>`).
const ANY: &str = "*";
const ANY_PREFIX: &str = "*:";
/// The languages the bot speaks out of the box (a spoken name is the same in all of them).
const BUILTIN_LANGS: [&str; 2] = ["de", "en"];

fn scope_of(scope: &str, id: &str) -> Option<Scope> {
    match scope {
        "global" => Some(Scope::Global),
        "server" => id.parse().ok().map(|guild| Scope::Server { guild }),
        "person" => {
            let (g, u) = id.split_once(':')?;
            Some(Scope::Person {
                guild: g.parse().ok()?,
                user: u.parse().ok()?,
            })
        }
        _ => None,
    }
}

fn scope_name(s: Scope) -> String {
    match s {
        Scope::Global => "global".into(),
        Scope::Server { guild } => format!("community {guild}"),
        Scope::Person { guild, user } => format!("person {user} in community {guild}"),
    }
}

/// The language of a Piper voice id (`de_DE-thorsten-medium` → `de`).
pub(crate) fn voice_lang(voice: &str) -> Option<Lang> {
    voice.split(['_', '-']).next().and_then(|l| l.parse().ok())
}

/// What the import did with the settings.
#[derive(Debug, Default)]
pub struct SettingsImport {
    pub changes: Vec<Change>,
    pub notes: Vec<String>,
    /// Old rows read.
    pub rows: u64,
}

struct Ctx<'a> {
    tree: &'a mut SettingsTree,
    out: SettingsImport,
}

impl Ctx<'_> {
    fn set(&mut self, scope: Scope, key: SettingKey, value: Value, old_key: &str) {
        match self.tree.set(scope, key, value.clone(), true) {
            Ok(Some(c)) => self.out.changes.push(c),
            Ok(None) => {}
            Err(e) => self.out.notes.push(format!(
                "{old_key} ({}): {value} was not carried over: {e}",
                scope_name(scope)
            )),
        }
    }

    fn removed(&mut self, scope: Scope, key: &str, value: &Value, why: &str) {
        self.out.notes.push(format!(
            "{key} = {value} ({}) was not carried over: {why}",
            scope_name(scope)
        ));
    }
}

fn secs(v: &Value, unit: f64) -> Value {
    match v.as_f64() {
        Some(x) => json!(x / unit),
        None => json!("unlimited"),
    }
}

fn line_for(t: &str, step: Option<&str>) -> Option<LineKey> {
    let label = if t == "any" {
        Sel::Any
    } else {
        Sel::Is(t.parse::<Label>().ok()?)
    };
    let step = match step {
        None => Sel::Any,
        Some(s) => Sel::Is(s.parse::<u32>().ok()?),
    };
    Some(LineKey(Line::Warning { label, step }))
}

/// Converts the old settings and tracked people into changes of `tree`.
pub fn convert(old: &OldDb, tree: &mut SettingsTree, clips: &BTreeMap<String, BlobHash>) -> SettingsImport {
    let mut by_scope: BTreeMap<Scope, BTreeMap<String, Value>> = BTreeMap::new();
    // The old bot's person settings for every community (`*:<user>`).
    let mut everywhere: BTreeMap<UserId, BTreeMap<String, Value>> = BTreeMap::new();
    let mut rows = 0;
    let mut notes = Vec::new();
    for r in &old.settings {
        rows += 1;
        let (scope_kind, id) = (r.text(0).unwrap_or_default(), r.text(1).unwrap_or_default());
        if let (Some(user), Some(key)) = (
            id.strip_prefix(ANY_PREFIX)
                .filter(|_| scope_kind == "person")
                .and_then(|u| u.parse().ok()),
            r.text(2),
        ) {
            let value = r
                .text(3)
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            everywhere.entry(user).or_default().insert(key, value);
            continue;
        }
        let (Some(scope), Some(key)) = (
            scope_of(&r.text(0).unwrap_or_default(), &r.text(1).unwrap_or_default()),
            r.text(2),
        ) else {
            notes.push(format!("a settings row with an unknown scope was skipped: {:?}", r.0));
            continue;
        };
        let value = r
            .text(3)
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        by_scope.entry(scope).or_default().insert(key, value);
    }
    // A person's settings for every community apply in each community they are tracked in or have settings in, under
    // the ones for that community (as the old bot layered them). Communities they join later do not get them.
    let communities_of = |user: UserId, by_scope: &BTreeMap<Scope, BTreeMap<String, Value>>| {
        let mut gs: Vec<GuildId> = old
            .tracked
            .iter()
            .filter(|r| r.text(1).and_then(|u| u.parse::<UserId>().ok()) == Some(user))
            .filter_map(|r| r.text(0).and_then(|g| g.parse().ok()))
            .chain(by_scope.keys().filter_map(|s| match s {
                Scope::Person { guild, user: u } if *u == user => Some(*guild),
                _ => None,
            }))
            .collect();
        gs.sort();
        gs.dedup();
        gs
    };
    for (user, keys) in everywhere {
        let guilds = communities_of(user, &by_scope);
        notes.push(format!(
            "settings of {user} for every community ({}) were applied in the communities they are tracked in or have              settings in ({}); they do not apply in other communities",
            keys.keys().cloned().collect::<Vec<_>>().join(", "),
            if guilds.is_empty() {
                "none".to_owned()
            } else {
                guilds.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            }
        ));
        for guild in guilds {
            let own = by_scope.entry(Scope::Person { guild, user }).or_default();
            for (k, v) in &keys {
                own.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    let voice_of = |s: Scope| {
        by_scope
            .get(&s)
            .and_then(|m| m.get("voice"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let lang_at = |s: Scope| -> Lang {
        let chain = match s {
            Scope::Global => vec![Scope::Global],
            Scope::Server { .. } => vec![s, Scope::Global],
            Scope::Person { guild, .. } => vec![s, Scope::Server { guild }, Scope::Global],
        };
        chain
            .into_iter()
            .find_map(voice_of)
            .and_then(|v| voice_lang(&v))
            .or_else(|| voice_lang(OLD_DEFAULT_VOICE))
            .unwrap_or_else(|| unreachable!("the default voice has a language"))
    };
    let mut cx = Ctx {
        tree,
        out: SettingsImport {
            rows,
            notes,
            ..SettingsImport::default()
        },
    };
    for (&scope, keys) in &by_scope {
        let lang = lang_at(scope);
        let mut lines: BTreeMap<LineKey, Slot> = BTreeMap::new();
        for (key, v) in keys {
            use SettingKey as K;
            let simple = |k: &str| -> Option<SettingKey> {
                Some(match k {
                    "paused" => K::Paused,
                    "guild_ids" => K::GuildAllowlist,
                    "allow_e2ee_downgrade" => K::AllowE2eeDowngrade,
                    "threshold" => K::Threshold,
                    "strikes" => K::Strikes,
                    "observe_only" => K::ObserveOnly,
                    "audience" => K::Audience,
                    "volume_db" => K::VolumeDb,
                    "no_speak_policy" => K::NoSpeakPolicy,
                    "actions_enabled" => K::ActionsEnabled,
                    "greet_enabled" => K::GreetEnabled,
                    "modlog_audio" => K::ModlogAudio,
                    "owner_dm_audio" => K::OwnerDmAudio,
                    "digest" => K::Digest,
                    "digest_time" => K::DigestTime,
                    "timezone" => K::Timezone,
                    "jar_enabled" => K::JarEnabled,
                    "admins_play_audio" => K::AdminsPlayAudio,
                    "commands" => K::CommandsEnabled,
                    "command_prefix" => K::CommandPrefix,
                    "admin_user_ids" => K::AdminUserIds,
                    "admin_role_ids" => K::AdminRoleIds,
                    "instance" => K::Instance,
                    "cpu_threads" => K::CpuThreads,
                    _ => {
                        return k
                            .parse::<SettingKey>()
                            .ok()
                            .filter(|k| matches!(k, K::LabelEnabled(_) | K::LabelThreshold(_)));
                    }
                })
            };
            match key.as_str() {
                "max_per_hour" => cx.removed(scope, key, v, "warnings are no longer capped per hour"),
                "max_channels" => cx.removed(
                    scope,
                    key,
                    v,
                    "the bot is no longer limited to a number of voice channels",
                ),
                "history_days" | "evidence_days" | "audit_days" => cx.removed(
                    scope,
                    key,
                    v,
                    "everything is kept now (recordings can still be deleted one by one)",
                ),
                "recent_clips" => cx.removed(
                    scope,
                    key,
                    v,
                    "the live view keeps its recent sentences by itself; all of them are in History",
                ),
                "join_settle_s" => cx.set(scope, K::JoinSettle, secs(v, 1.0), key),
                "leave_grace_s" => cx.set(scope, K::LeaveGrace, secs(v, 1.0), key),
                "strike_window_s" => cx.set(scope, K::StrikeWindow, secs(v, 1.0), key),
                "violation_window_s" => cx.set(scope, K::ViolationWindow, secs(v, 1.0), key),
                "end_silence_ms" => cx.set(scope, K::EndSilence, secs(v, 1000.0), key),
                "min_voiced_ms" => cx.set(scope, K::MinVoiced, secs(v, 1000.0), key),
                "max_clip_s" => cx.set(scope, K::MaxSentence, secs(v, 1.0), key),
                "modlog_channel" | "ui_url" if v.as_str().is_some_and(str::is_empty) || v.is_null() => {}
                "modlog_channel" => cx.set(scope, K::ModlogChannel, v.clone(), key),
                "ui_url" => cx.set(scope, K::UiUrl, v.clone(), key),
                "log_level" => cx.removed(
                    scope,
                    key,
                    v,
                    "the log level is set in config.toml ([logging] level) or with RUST_LOG",
                ),
                "evidence_enabled" => cx.set(
                    scope,
                    K::Recordings,
                    json!(if v.as_bool().unwrap_or(true) { "flagged" } else { "off" }),
                    key,
                ),
                // Merged into the escalation steps below.
                "owner_dm" | "escalation" => {}
                "digest_weekday" => {
                    let names = [
                        "monday",
                        "tuesday",
                        "wednesday",
                        "thursday",
                        "friday",
                        "saturday",
                        "sunday",
                    ];
                    match v.as_f64().map(|d| d as usize).and_then(|d| names.get(d)) {
                        Some(n) => cx.set(scope, K::DigestWeekday, json!(n), key),
                        None => cx.removed(scope, key, v, "not a day of the week"),
                    }
                }
                "voice" => {
                    let Some(voice) = v.as_str().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    match voice_lang(voice) {
                        Some(l) => {
                            cx.set(scope, K::VoiceLanguage, json!(l.to_string()), key);
                            cx.set(scope, K::TtsVoices, json!({ l.to_string(): voice }), key);
                        }
                        None => cx.removed(scope, key, v, "the voice's language is not recognisable"),
                    }
                }
                "spoken_name" => {
                    if let Some(name) = v.as_str().filter(|s| !s.is_empty()) {
                        let slot = lines.entry(LineKey(Line::Name)).or_default();
                        let mut langs: Vec<Lang> = BUILTIN_LANGS.iter().filter_map(|l| l.parse().ok()).collect();
                        if !langs.contains(&lang) {
                            langs.push(lang.clone());
                        }
                        for l in langs {
                            slot.text.insert(l, name.to_owned());
                        }
                    }
                }
                "greet_text" => {
                    if let Some(t) = v.as_str().filter(|s| !s.is_empty()) {
                        lines
                            .entry(LineKey(Line::Greeting))
                            .or_default()
                            .text
                            .insert(lang.clone(), t.to_owned());
                    }
                }
                "warn_texts" => {
                    for (t, steps) in v.as_object().into_iter().flatten() {
                        for (step, text) in steps.as_object().into_iter().flatten() {
                            match (line_for(t, Some(step)), text.as_str()) {
                                (Some(k), Some(text)) => {
                                    lines.entry(k).or_default().text.insert(lang.clone(), text.to_owned());
                                }
                                _ => cx.out.notes.push(format!(
                                    "warn_texts {t}/{step} ({}) was not carried over",
                                    scope_name(scope)
                                )),
                            }
                        }
                    }
                }
                "clips" => {
                    for (t, cell) in v.as_object().into_iter().flatten() {
                        let targets: Vec<(LineKey, &Value)> = if t == "greeting" {
                            vec![(LineKey(Line::Greeting), cell)]
                        } else {
                            cell.as_object()
                                .into_iter()
                                .flatten()
                                .filter_map(|(step, ids)| line_for(t, Some(step)).map(|k| (k, ids)))
                                .collect()
                        };
                        for (k, ids) in targets {
                            for id in ids.as_array().into_iter().flatten().filter_map(Value::as_str) {
                                match clips.get(id) {
                                    Some(h) => {
                                        let slot = lines.entry(k.clone()).or_default();
                                        if !slot.clips.contains(h) {
                                            slot.clips.push(*h);
                                        }
                                    }
                                    None => cx.out.notes.push(format!(
                                        "clip {id} ({}, {k}) was not found and is left out",
                                        scope_name(scope)
                                    )),
                                }
                            }
                        }
                    }
                }
                other => match simple(other) {
                    Some(k) => cx.set(scope, k, v.clone(), key),
                    None => cx.removed(scope, key, v, "this setting does not exist any more"),
                },
            }
        }
        for (k, slot) in lines {
            if let Some(c) = cx.tree.set_voice_line(scope, k, Some(slot)) {
                cx.out.changes.push(c);
            }
        }
    }
    // The old bot told the owner about every violation (`owner_dm`, on by default) and also when a step said so; now
    // only the steps do. Wherever either was set, the steps that apply there get both.
    let chain = |s: Scope| match s {
        Scope::Global => vec![Scope::Global],
        Scope::Server { .. } => vec![s, Scope::Global],
        Scope::Person { guild, .. } => vec![s, Scope::Server { guild }, Scope::Global],
    };
    let nearest = |s: Scope, key: &str| chain(s).into_iter().find_map(|c| by_scope.get(&c)?.get(key).cloned());
    let mut modlog_steps = false;
    for (&scope, keys) in &by_scope {
        if !keys.contains_key("owner_dm") && !keys.contains_key("escalation") {
            continue;
        }
        let owner_dm = nearest(scope, "owner_dm").and_then(|v| v.as_bool()).unwrap_or(true);
        let old_steps = nearest(scope, "escalation")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_else(|| {
                vec![
                    json!({"from": 1}),
                    json!({"from": 2}),
                    json!({"from": 3, "notify_owner": true, "notify_modlog": true}),
                ]
            });
        let steps: Vec<Value> = old_steps
            .iter()
            .map(|s| {
                let flag = |k: &str| s.get(k).and_then(Value::as_bool).unwrap_or(false);
                modlog_steps |= flag("notify_modlog");
                json!({
                    "from": s.get("from").cloned().unwrap_or(json!(1)),
                    "notify_owner": owner_dm || flag("notify_owner"),
                    "action": s.get("action").cloned().unwrap_or(json!("none")),
                    "duration": s.get("minutes").and_then(Value::as_f64).unwrap_or(5.0) * 60.0,
                })
            })
            .collect();
        cx.set(scope, SettingKey::Escalation, Value::Array(steps), "escalation");
    }
    if modlog_steps {
        cx.out.notes.push(
            "escalation posts in the mod log are now part of the post every flagged sentence gets (with the step and \
             the action's result)"
                .to_owned(),
        );
    }
    let mut tracked_everywhere: Vec<UserId> = cx.tree.effective(None, None).tracked_everywhere.value.clone();
    for r in &old.tracked {
        if r.text(0).as_deref() == Some(ANY) {
            match r.text(1).and_then(|u| u.parse::<UserId>().ok()) {
                Some(user) if !tracked_everywhere.contains(&user) => tracked_everywhere.push(user),
                Some(_) => {}
                None => cx.out.notes.push(format!("a tracked row was skipped: {:?}", r.0)),
            }
            continue;
        }
        let (Some(guild), Some(user)) = (
            r.text(0).and_then(|g| g.parse::<GuildId>().ok()),
            r.text(1).and_then(|u| u.parse::<UserId>().ok()),
        ) else {
            cx.out.notes.push(format!("a tracked row was skipped: {:?}", r.0));
            continue;
        };
        let by = r.text(2).and_then(|b| b.parse::<UserId>().ok());
        let at = r
            .real(3)
            .and_then(|t| Timestamp::from_millisecond((t * 1000.0) as i64).ok())
            .unwrap_or(Timestamp::UNIX_EPOCH);
        if let Some(c) = cx.tree.track(guild, user, by, at) {
            cx.out.changes.push(c);
        }
    }
    // People tracked in every community.
    if tracked_everywhere != cx.tree.effective(None, None).tracked_everywhere.value {
        let ids: Vec<String> = tracked_everywhere.iter().map(ToString::to_string).collect();
        cx.set(Scope::Global, SettingKey::TrackedEverywhere, json!(ids), "tracked *");
    }
    cx.out
}
