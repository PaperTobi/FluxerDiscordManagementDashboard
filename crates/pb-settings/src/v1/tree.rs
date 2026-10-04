//! Every scope's settings, tracked people and voice lines, and the changes made to them.

use std::collections::{BTreeMap, BTreeSet};

use pb_domain::{GuildId, Scope, ScopeKind, UserId};
use pb_voicelines::{LineKey, ScopedSlots, Slot, Slots};
use serde::{Deserialize, Serialize};

use super::schema::{Effective, Layer, Layers, Resolved, SettingError, SettingKey, Source, Who, resolve, set};

/// Current version of the settings file format.
pub const SCHEMA_VERSION: u32 = 1;

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

/// `settings/global.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GlobalFile {
    #[serde(default = "schema_version")]
    pub schema: u32,
    pub settings: Layer,
    pub voice_lines: Slots,
}

/// `settings/servers/<community id>.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerFile {
    #[serde(default = "schema_version")]
    pub schema: u32,
    pub settings: Layer,
    pub voice_lines: Slots,
    pub people: BTreeMap<UserId, PersonEntry>,
}

impl Default for GlobalFile {
    fn default() -> Self {
        GlobalFile {
            schema: SCHEMA_VERSION,
            settings: Layer::default(),
            voice_lines: Slots::default(),
        }
    }
}

impl Default for ServerFile {
    fn default() -> Self {
        ServerFile {
            schema: SCHEMA_VERSION,
            settings: Layer::default(),
            voice_lines: Slots::default(),
            people: BTreeMap::new(),
        }
    }
}

/// A person in a community: tracked or not, and their own settings and voice lines.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonEntry {
    pub tracked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_by: Option<UserId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_at: Option<jiff::Timestamp>,
    pub settings: Layer,
    pub voice_lines: Slots,
}

impl PersonEntry {
    fn is_empty(&self) -> bool {
        !self.tracked && self.settings == Layer::default() && self.voice_lines.is_empty()
    }
}

/// A change to the settings, for the audit log and for editing the files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Set {
        scope: Scope,
        key: String,
        before: Option<serde_json::Value>,
        after: serde_json::Value,
    },
    Clear {
        scope: Scope,
        key: String,
        before: Option<serde_json::Value>,
    },
    Track {
        guild: GuildId,
        user: UserId,
        by: Option<UserId>,
        at: jiff::Timestamp,
    },
    Untrack {
        guild: GuildId,
        user: UserId,
    },
    VoiceLine {
        scope: Scope,
        line: LineKey,
        before: Option<Slot>,
        after: Option<Slot>,
    },
}

impl Change {
    /// The community whose file this change edits (`None` = the global file).
    pub fn guild(&self) -> Option<GuildId> {
        match self {
            Change::Set { scope, .. } | Change::Clear { scope, .. } | Change::VoiceLine { scope, .. } => scope.guild(),
            Change::Track { guild, .. } | Change::Untrack { guild, .. } => Some(*guild),
        }
    }
}

/// All settings of the bot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsTree {
    /// `config.toml` `[defaults]` and environment variables.
    pub file_defaults: Layer,
    pub global: GlobalFile,
    pub servers: BTreeMap<GuildId, ServerFile>,
}

impl SettingsTree {
    fn layer(&self, scope: Scope) -> Option<&Layer> {
        match scope {
            Scope::Global => Some(&self.global.settings),
            Scope::Server { guild } => self.servers.get(&guild).map(|s| &s.settings),
            Scope::Person { guild, user } => self.servers.get(&guild)?.people.get(&user).map(|p| &p.settings),
        }
    }

    fn layer_mut(&mut self, scope: Scope) -> &mut Layer {
        match scope {
            Scope::Global => &mut self.global.settings,
            Scope::Server { guild } => &mut self.servers.entry(guild).or_default().settings,
            Scope::Person { guild, user } => {
                &mut self
                    .servers
                    .entry(guild)
                    .or_default()
                    .people
                    .entry(user)
                    .or_default()
                    .settings
            }
        }
    }

    /// Settings set exactly at `scope`.
    pub fn overrides(&self, scope: Scope) -> Layer {
        self.layer(scope).cloned().unwrap_or_default()
    }

    /// Everything resolved for a place. A pause of the community also pauses everyone in it.
    /// The layers that decide a place's settings, most specific first.
    fn layers(&self, guild: Option<GuildId>, user: Option<UserId>) -> Layers<'_> {
        Layers {
            person: guild
                .zip(user)
                .and_then(|(g, u)| self.layer(Scope::Person { guild: g, user: u })),
            server: guild.and_then(|g| self.layer(Scope::Server { guild: g })),
            global: Some(&self.global.settings),
            file: Some(&self.file_defaults),
        }
    }

    pub fn effective(&self, guild: Option<GuildId>, user: Option<UserId>) -> Effective {
        let layers = self.layers(guild, user);
        let (person, server) = (layers.person, layers.server);
        let mut e = resolve(&layers);
        if person.is_some() || user.is_some() {
            let community = resolve(&Layers {
                person: None,
                server,
                global: Some(&self.global.settings),
                file: Some(&self.file_defaults),
            });
            if community.paused.value && !e.paused.value {
                e.paused = Resolved {
                    value: true,
                    source: community.paused.source,
                };
            }
        }
        e
    }

    pub fn guild_allowed(&self, guild: GuildId) -> bool {
        let allow = self.effective(None, None).guild_allowlist.value;
        allow.is_empty() || allow.contains(&guild)
    }

    /// Everyone on the community's list (tracked there or in every community), paused or not.
    pub fn listed_for(&self, guild: GuildId) -> BTreeSet<UserId> {
        let mut out: BTreeSet<UserId> = self
            .effective(None, None)
            .tracked_everywhere
            .value
            .into_iter()
            .collect();
        if let Some(s) = self.servers.get(&guild) {
            out.extend(s.people.iter().filter(|(_, p)| p.tracked).map(|(u, _)| *u));
        }
        out
    }

    /// Who the bot follows in the community right now (nobody while the community is paused or not allowed).
    pub fn tracked_for(&self, guild: GuildId) -> BTreeSet<UserId> {
        if !self.guild_allowed(guild) || self.effective(Some(guild), None).paused.value {
            return BTreeSet::new();
        }
        self.listed_for(guild)
            .into_iter()
            .filter(|u| !self.effective(Some(guild), Some(*u)).paused.value)
            .collect()
    }

    pub fn is_tracked(&self, guild: GuildId, user: UserId) -> bool {
        self.guild_allowed(guild)
            && !self.effective(Some(guild), None).paused.value
            && self.listed_for(guild).contains(&user)
            && !self.effective(Some(guild), Some(user)).paused.value
    }

    /// Communities that have a settings file.
    pub fn known_guilds(&self) -> BTreeSet<GuildId> {
        self.servers.keys().copied().collect()
    }

    /// Voice lines set exactly at `scope`.
    pub fn voice_lines(&self, scope: Scope) -> Option<&Slots> {
        match scope {
            Scope::Global => Some(&self.global.voice_lines),
            Scope::Server { guild } => self.servers.get(&guild).map(|s| &s.voice_lines),
            Scope::Person { guild, user } => self.servers.get(&guild)?.people.get(&user).map(|p| &p.voice_lines),
        }
    }

    /// The voice-line slots of the three scopes for a person in a community.
    pub fn scoped_slots(&self, guild: GuildId, user: Option<UserId>) -> ScopedSlots<'_> {
        ScopedSlots {
            person: user.and_then(|u| self.voice_lines(Scope::Person { guild, user: u })),
            server: self.voice_lines(Scope::Server { guild }),
            global: Some(&self.global.voice_lines),
        }
    }

    /// Validates and sets a setting; returns the change (not applied when it changes nothing).
    pub fn set(
        &mut self,
        scope: Scope,
        key: SettingKey,
        value: serde_json::Value,
        by_owner: bool,
    ) -> Result<Option<Change>, SettingError> {
        let before = self.layer(scope).and_then(|l| l.get_json(key));
        let mut layer = self.layer(scope).cloned().unwrap_or_default();
        let after = set(&mut layer, scope.kind(), key, value, by_owner)?;
        if after.is_null() {
            return Ok(self.remove(scope, key));
        }
        if before.as_ref() == Some(&after) {
            return Ok(None);
        }
        *self.layer_mut(scope) = layer;
        Ok(Some(Change::Set {
            scope,
            key: key.name(),
            before,
            after,
        }))
    }

    /// Removes a setting at `scope` (it is inherited again). Only the owner removes what only the owner may set.
    pub fn clear(&mut self, scope: Scope, key: SettingKey, by_owner: bool) -> Result<Option<Change>, SettingError> {
        if key.who() == Who::Owner && !by_owner {
            return Err(SettingError::OwnerOnly { key: key.name() });
        }
        Ok(self.remove(scope, key))
    }

    fn remove(&mut self, scope: Scope, key: SettingKey) -> Option<Change> {
        let before = self.layer(scope).and_then(|l| l.get_json(key))?;
        self.layer_mut(scope).clear(key);
        self.prune(scope);
        Some(Change::Clear {
            scope,
            key: key.name(),
            before: Some(before),
        })
    }

    /// Removes every setting at `scope` except `keep` (and, unless `by_owner`, those only the owner may change).
    pub fn reset(&mut self, scope: Scope, keep: &[SettingKey], by_owner: bool) -> Vec<Change> {
        let keys = self.layer(scope).map(Layer::keys).unwrap_or_default();
        keys.into_iter()
            .filter(|k| !keep.contains(k) && (by_owner || k.who() != Who::Owner))
            .filter_map(|k| self.remove(scope, k))
            .collect()
    }

    pub fn track(&mut self, guild: GuildId, user: UserId, by: Option<UserId>, at: jiff::Timestamp) -> Option<Change> {
        let entry = self.servers.entry(guild).or_default().people.entry(user).or_default();
        if entry.tracked {
            return None;
        }
        entry.tracked = true;
        entry.added_by = by;
        entry.added_at = Some(at);
        Some(Change::Track { guild, user, by, at })
    }

    pub fn untrack(&mut self, guild: GuildId, user: UserId) -> Option<Change> {
        let entry = self.servers.get_mut(&guild)?.people.get_mut(&user)?;
        if !entry.tracked {
            return None;
        }
        entry.tracked = false;
        entry.added_by = None;
        entry.added_at = None;
        self.prune(Scope::Person { guild, user });
        Some(Change::Untrack { guild, user })
    }

    /// Sets (or with `None` removes) a voice-line slot at `scope`.
    pub fn set_voice_line(&mut self, scope: Scope, line: LineKey, slot: Option<Slot>) -> Option<Change> {
        let slot = slot.filter(|s| !s.is_empty());
        let lines = match scope {
            Scope::Global => &mut self.global.voice_lines,
            Scope::Server { guild } => &mut self.servers.entry(guild).or_default().voice_lines,
            Scope::Person { guild, user } => {
                &mut self
                    .servers
                    .entry(guild)
                    .or_default()
                    .people
                    .entry(user)
                    .or_default()
                    .voice_lines
            }
        };
        let before = match &slot {
            Some(s) => lines.insert(line.clone(), s.clone()),
            None => lines.remove(&line),
        };
        if before == slot {
            return None;
        }
        self.prune(scope);
        Some(Change::VoiceLine {
            scope,
            line,
            before,
            after: slot,
        })
    }

    fn prune(&mut self, scope: Scope) {
        if let Scope::Person { guild, user } = scope
            && let Some(s) = self.servers.get_mut(&guild)
            && s.people.get(&user).is_some_and(PersonEntry::is_empty)
        {
            s.people.remove(&user);
        }
    }

    /// Checks every layer for settings that may not be set at its scope.
    pub fn misplaced(&self) -> Vec<(Scope, SettingKey)> {
        let mut out: Vec<(Scope, SettingKey)> = self
            .global
            .settings
            .misplaced(ScopeKind::Global)
            .into_iter()
            .map(|k| (Scope::Global, k))
            .collect();
        for (g, s) in &self.servers {
            out.extend(
                s.settings
                    .misplaced(ScopeKind::Server)
                    .into_iter()
                    .map(|k| (Scope::Server { guild: *g }, k)),
            );
            for (u, p) in &s.people {
                out.extend(
                    p.settings
                        .misplaced(ScopeKind::Person)
                        .into_iter()
                        .map(|k| (Scope::Person { guild: *g, user: *u }, k)),
                );
            }
        }
        out
    }

    /// Where a setting comes from for a place, for "inherited from" badges.
    pub fn source_of(&self, guild: Option<GuildId>, user: Option<UserId>, key: SettingKey) -> Source {
        self.layers(guild, user)
            .ordered()
            .into_iter()
            .find(|(_, l)| l.get_json(key).is_some())
            .map_or(Source::Default, |(s, _)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: GuildId = GuildId(10);
    const A: UserId = UserId(1);
    const B: UserId = UserId(2);

    fn now() -> jiff::Timestamp {
        jiff::Timestamp::UNIX_EPOCH
    }

    #[test]
    fn tracking_and_pausing_follow_the_old_rules() {
        let mut t = SettingsTree::default();
        assert!(t.track(G, A, None, now()).is_some());
        assert!(t.track(G, A, None, now()).is_none(), "already tracked");
        t.track(G, B, None, now());
        assert_eq!(t.tracked_for(G).len(), 2);
        t.set(
            Scope::Person { guild: G, user: B },
            SettingKey::Paused,
            serde_json::json!(true),
            false,
        )
        .expect("pause B");
        assert_eq!(t.tracked_for(G), BTreeSet::from([A]));
        assert_eq!(t.listed_for(G).len(), 2, "paused people stay listed");
        t.set(
            Scope::Server { guild: G },
            SettingKey::Paused,
            serde_json::json!(true),
            false,
        )
        .expect("pause server");
        assert!(t.tracked_for(G).is_empty());
        assert!(
            t.effective(Some(G), Some(A)).paused.value,
            "a server pause pauses the person"
        );
        t.set(
            Scope::Global,
            SettingKey::TrackedEverywhere,
            serde_json::json!(["3"]),
            true,
        )
        .expect("everywhere");
        t.clear(Scope::Server { guild: G }, SettingKey::Paused, false).unwrap();
        assert!(t.tracked_for(G).contains(&UserId(3)));
        t.set(
            Scope::Global,
            SettingKey::GuildAllowlist,
            serde_json::json!(["99"]),
            true,
        )
        .expect("allow list");
        assert!(t.tracked_for(G).is_empty(), "not an allowed community");
    }

    #[test]
    fn untracked_people_without_settings_disappear() {
        let mut t = SettingsTree::default();
        t.track(G, A, Some(B), now());
        assert!(t.untrack(G, A).is_some());
        assert!(t.servers[&G].people.is_empty());
        t.track(G, A, None, now());
        t.set(
            Scope::Person { guild: G, user: A },
            SettingKey::Strikes,
            serde_json::json!(2),
            false,
        )
        .expect("set");
        t.untrack(G, A);
        assert!(t.servers[&G].people.contains_key(&A), "kept for their settings");
    }

    #[test]
    fn changes_are_reported_only_when_something_changes() {
        let mut t = SettingsTree::default();
        let c = t
            .set(Scope::Global, SettingKey::Strikes, serde_json::json!(3), false)
            .expect("set");
        assert!(matches!(c, Some(Change::Set { ref before, .. }) if before.is_none()));
        assert_eq!(
            t.set(Scope::Global, SettingKey::Strikes, serde_json::json!(3), false)
                .expect("same"),
            None
        );
        assert!(
            t.set(
                Scope::Server { guild: G },
                SettingKey::ModlogChannel,
                serde_json::Value::Null,
                false
            )
            .expect("empty")
            .is_none()
        );
        assert_eq!(t.source_of(Some(G), None, SettingKey::Strikes), Source::Global);
        let resets = t.reset(Scope::Global, &[], true);
        assert_eq!(resets.len(), 1);
    }

    #[test]
    fn only_the_owner_removes_owner_only_settings() {
        let mut t = SettingsTree::default();
        let server = Scope::Server { guild: G };
        t.set(server, SettingKey::ModlogAudio, serde_json::json!(true), true)
            .expect("owner sets");
        t.set(server, SettingKey::Strikes, serde_json::json!(2), false)
            .expect("admin sets");
        assert!(matches!(
            t.clear(server, SettingKey::ModlogAudio, false),
            Err(SettingError::OwnerOnly { .. })
        ));
        let resets = t.reset(server, &[], false);
        assert_eq!(resets.len(), 1, "an admin's reset leaves owner-only settings");
        assert_eq!(t.source_of(Some(G), None, SettingKey::ModlogAudio), Source::Server);
        assert!(
            t.clear(server, SettingKey::ModlogAudio, true)
                .expect("owner clears")
                .is_some()
        );
    }
}
