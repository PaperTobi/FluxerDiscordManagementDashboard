//! A settings tree with every place's resolved settings worked out once. A tree that is shared does not change, so
//! nothing here goes stale: a change builds a new view.

use std::collections::{BTreeSet, HashMap};
use std::ops::Deref;
use std::sync::Arc;

use pb_domain::{GuildId, UserId};

use super::{Effective, SettingsTree};

/// A community's resolved settings and who is on its list.
#[derive(Debug)]
struct ServerView {
    effective: Arc<Effective>,
    listed: Arc<BTreeSet<UserId>>,
    tracked: Arc<BTreeSet<UserId>>,
}

/// The settings, resolved: the same answers as [`SettingsTree`]'s methods, without resolving the layers again on
/// every call. Everything else of the tree is reached through `Deref`.
#[derive(Debug)]
pub struct SettingsView {
    tree: SettingsTree,
    global: Arc<Effective>,
    /// A community without a settings file: the global settings and only those tracked everywhere.
    no_file: ServerView,
    servers: HashMap<GuildId, ServerView>,
    /// Only people with settings of their own (everyone else has their community's).
    people: HashMap<(GuildId, UserId), Arc<Effective>>,
}

impl SettingsView {
    pub fn new(tree: SettingsTree) -> SettingsView {
        let global = Arc::new(tree.effective(None, None));
        let everywhere: BTreeSet<UserId> = global.tracked_everywhere.value.iter().copied().collect();
        let no_file = ServerView {
            effective: global.clone(),
            tracked: Arc::new(if global.paused.value {
                BTreeSet::new()
            } else {
                everywhere.clone()
            }),
            listed: Arc::new(everywhere),
        };
        let mut servers = HashMap::new();
        let mut people = HashMap::new();
        for (&g, file) in &tree.servers {
            servers.insert(
                g,
                ServerView {
                    effective: Arc::new(tree.effective(Some(g), None)),
                    listed: Arc::new(tree.listed_for(g)),
                    tracked: Arc::new(tree.tracked_for(g)),
                },
            );
            for &u in file.people.keys() {
                people.insert((g, u), Arc::new(tree.effective(Some(g), Some(u))));
            }
        }
        SettingsView {
            tree,
            global,
            no_file,
            servers,
            people,
        }
    }

    pub fn tree(&self) -> &SettingsTree {
        &self.tree
    }

    fn server(&self, guild: GuildId) -> &ServerView {
        self.servers.get(&guild).unwrap_or(&self.no_file)
    }

    /// Everything resolved for a place (see [`SettingsTree::effective`]).
    pub fn effective(&self, guild: Option<GuildId>, user: Option<UserId>) -> Arc<Effective> {
        match (guild, user) {
            (None, _) => self.global.clone(),
            (Some(g), Some(u)) => match self.people.get(&(g, u)) {
                Some(e) => e.clone(),
                None => self.server(g).effective.clone(),
            },
            (Some(g), None) => self.server(g).effective.clone(),
        }
    }

    pub fn guild_allowed(&self, guild: GuildId) -> bool {
        let allow = &self.global.guild_allowlist.value;
        allow.is_empty() || allow.contains(&guild)
    }

    /// Everyone on the community's list (see [`SettingsTree::listed_for`]).
    pub fn listed_for(&self, guild: GuildId) -> Arc<BTreeSet<UserId>> {
        self.server(guild).listed.clone()
    }

    /// Who the bot follows in the community right now (see [`SettingsTree::tracked_for`]).
    pub fn tracked_for(&self, guild: GuildId) -> Arc<BTreeSet<UserId>> {
        if self.guild_allowed(guild) {
            self.server(guild).tracked.clone()
        } else {
            Arc::default()
        }
    }

    pub fn is_tracked(&self, guild: GuildId, user: UserId) -> bool {
        self.tracked_for(guild).contains(&user)
    }
}

impl Deref for SettingsView {
    type Target = SettingsTree;

    fn deref(&self) -> &SettingsTree {
        &self.tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb_domain::Scope;

    use crate::SettingKey;

    const G: GuildId = GuildId(10);
    const H: GuildId = GuildId(11);
    const UNKNOWN: GuildId = GuildId(12);
    const A: UserId = UserId(1);
    const B: UserId = UserId(2);
    const C: UserId = UserId(3);

    fn set(t: &mut SettingsTree, scope: Scope, key: SettingKey, value: serde_json::Value) {
        t.set(scope, key, value, true).expect("valid setting");
    }

    /// Every answer equals the tree's, for every place.
    fn same_as_tree(t: &SettingsTree) {
        let v = SettingsView::new(t.clone());
        let guilds = [None, Some(G), Some(H), Some(UNKNOWN)];
        let users = [None, Some(A), Some(B), Some(C)];
        for g in guilds {
            for u in users {
                assert_eq!(*v.effective(g, u), t.effective(g, u), "{g:?} {u:?}");
            }
        }
        for g in [G, H, UNKNOWN] {
            assert_eq!(v.guild_allowed(g), t.guild_allowed(g), "{g:?}");
            assert_eq!(*v.listed_for(g), t.listed_for(g), "{g:?}");
            assert_eq!(*v.tracked_for(g), t.tracked_for(g), "{g:?}");
            for u in [A, B, C] {
                assert_eq!(v.is_tracked(g, u), t.is_tracked(g, u), "{g:?} {u:?}");
            }
        }
    }

    #[test]
    fn answers_like_the_tree() {
        let now = jiff::Timestamp::UNIX_EPOCH;
        let mut t = SettingsTree::default();
        same_as_tree(&t);
        t.track(G, A, None, now);
        t.track(G, B, None, now);
        t.track(H, B, None, now);
        same_as_tree(&t);
        set(
            &mut t,
            Scope::Person { guild: G, user: B },
            SettingKey::Paused,
            serde_json::json!(true),
        );
        set(
            &mut t,
            Scope::Global,
            SettingKey::TrackedEverywhere,
            serde_json::json!(["3"]),
        );
        same_as_tree(&t);
        set(
            &mut t,
            Scope::Server { guild: H },
            SettingKey::Paused,
            serde_json::json!(true),
        );
        same_as_tree(&t);
        set(
            &mut t,
            Scope::Global,
            SettingKey::GuildAllowlist,
            serde_json::json!(["10", "12"]),
        );
        same_as_tree(&t);
    }
}
