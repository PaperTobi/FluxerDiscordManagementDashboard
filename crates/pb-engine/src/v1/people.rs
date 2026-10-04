//! The tracking list: the one way chat commands and the web UI add and remove people.

use std::sync::Arc;

use pb_domain::{GuildId, UserId};
use pb_store_api::{Actor, Event, PersonSeen};

use super::core::Core;
use super::engine::Engine;
use super::settings::ChangeError;

/// What tracking people did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tracked {
    pub added: Vec<UserId>,
    /// Already on the list (here or in every community).
    pub already: Vec<UserId>,
}

/// What untracking people did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Untracked {
    pub removed: Vec<UserId>,
    /// Tracked in every community: only the owner changes that (globally).
    pub everywhere: Vec<UserId>,
    /// Not on this community's list.
    pub missing: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TrackError {
    #[error("the bot does not track itself")]
    TheBot,
    #[error(transparent)]
    Change(#[from] ChangeError),
}

/// Puts people on a community's list and learns their names from Fluxer.
pub(crate) async fn track(
    core: &Arc<Core>,
    guild: GuildId,
    users: &[UserId],
    by: Actor,
) -> Result<Tracked, TrackError> {
    if core.bot().is_some_and(|b| users.contains(&b)) {
        return Err(TrackError::TheBot);
    }
    let listed = core.settings.current().listed_for(guild);
    let (already, added): (Vec<UserId>, Vec<UserId>) = users.iter().partition(|u| listed.contains(u));
    let (who, at, new) = (by.user, core.deps.clock.now(), added.clone());
    core.settings
        .change(by, move |t| {
            Ok(new.iter().filter_map(|u| t.track(guild, *u, who, at)).collect())
        })
        .await?;
    for u in &added {
        learn_person(core, guild, *u).await;
    }
    core.mark_guild(guild);
    Ok(Tracked { added, already })
}

/// Takes people off a community's list.
pub(crate) async fn untrack(
    core: &Arc<Core>,
    guild: GuildId,
    users: &[UserId],
    by: Actor,
) -> Result<Untracked, ChangeError> {
    let tree = core.settings.current();
    let everywhere = tree.effective(None, None).tracked_everywhere.value.clone();
    let here: Vec<UserId> = tree
        .servers
        .get(&guild)
        .map(|s| s.people.iter().filter(|(_, p)| p.tracked).map(|(u, _)| *u).collect())
        .unwrap_or_default();
    let mut out = Untracked::default();
    for u in users {
        if here.contains(u) {
            out.removed.push(*u);
        } else if everywhere.contains(u) {
            out.everywhere.push(*u);
        } else {
            out.missing.push(*u);
        }
    }
    let gone = out.removed.clone();
    core.settings
        .change(by, move |t| {
            Ok(gone.iter().filter_map(|u| t.untrack(guild, *u)).collect())
        })
        .await?;
    core.mark_guild(guild);
    Ok(out)
}

/// Asks Fluxer for a person's names in a community (when the bot is connected) and remembers them.
pub(crate) async fn learn_person(core: &Core, guild: GuildId, user: UserId) {
    let Some(ctl) = core.ctl() else { return };
    match ctl.member(guild, user).await {
        Ok(Some(m)) => {
            if let Some(u) = &m.user {
                let p = super::guilds::Person {
                    user,
                    username: u.username.clone(),
                    display_name: u.global_name.clone(),
                    nick: m.nick.clone(),
                    avatar: u.avatar.clone(),
                    roles: m.roles.clone(),
                    bot: u.bot,
                };
                core.update_guilds(|gs| gs.remember(guild, p));
                core.record(vec![Event::PersonSeen(PersonSeen {
                    user,
                    guild: Some(guild),
                    username: u.username.clone(),
                    display_name: u.global_name.clone(),
                    nick: m.nick.clone(),
                    avatar: u.avatar.clone(),
                })])
                .await;
                core.mark_person(guild, user);
            }
        }
        Ok(None) => tracing::info!(%guild, %user, "not a member of that community"),
        Err(e) => tracing::warn!(error = %e, "could not look up a member"),
    }
}

impl Engine {
    /// Puts people on a community's list (and learns their names from Fluxer).
    pub async fn track(&self, guild: GuildId, users: &[UserId], by: Actor) -> Result<Tracked, TrackError> {
        track(&self.core, guild, users, by).await
    }

    /// Takes people off a community's list.
    pub async fn untrack(&self, guild: GuildId, users: &[UserId], by: Actor) -> Result<Untracked, ChangeError> {
        untrack(&self.core, guild, users, by).await
    }
}
