//! Which voice channels to be in.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use pb_domain::{ChannelId, ConnectionId, GuildId, UserId};

use super::world::{VState, VoiceWorld};

/// A voice channel in a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chan {
    pub guild: GuildId,
    pub channel: ChannelId,
}

impl fmt::Display for Chan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.guild, self.channel)
    }
}

/// Mirrors Fluxer's rule (`guild_voice_e2ee.erl`, `channel_is_e2ee_active`): a channel is end-to-end encrypted when it
/// has at least one voice state and every one of them is `e2ee_capable` (the bot's own state is not counted).
pub fn e2ee_active(world: &VoiceWorld, chan: Chan, exclude: Option<UserId>) -> bool {
    let mut any = false;
    for v in world.in_channel(chan.guild, chan.channel) {
        if Some(v.user) == exclude {
            continue;
        }
        any = true;
        if !v.e2ee_capable {
            return false;
        }
    }
    any
}

/// `(channels to be in, channels skipped because they are end-to-end encrypted)`: every voice channel of an available
/// community that holds a person tracked there. There is no limit on how many.
pub fn desired_channels(
    world: &VoiceWorld,
    tracked: &dyn Fn(GuildId) -> BTreeSet<UserId>,
    bot: Option<UserId>,
    allow_e2ee_downgrade: &dyn Fn(GuildId) -> bool,
) -> (Vec<Chan>, Vec<Chan>) {
    let mut by_guild: BTreeMap<GuildId, BTreeSet<UserId>> = BTreeMap::new();
    let mut found: BTreeSet<Chan> = BTreeSet::new();
    for v in world.states() {
        if !world.available.contains(&v.guild) {
            continue;
        }
        let people = by_guild.entry(v.guild).or_insert_with(|| tracked(v.guild));
        if people.contains(&v.user) {
            found.insert(Chan {
                guild: v.guild,
                channel: v.channel,
            });
        }
    }
    let (mut want, mut blocked) = (Vec::new(), Vec::new());
    for chan in found {
        if !allow_e2ee_downgrade(chan.guild) && e2ee_active(world, chan, bot) {
            blocked.push(chan);
        } else {
            want.push(chan);
        }
    }
    (want, blocked)
}

/// Voice connections of the bot's own account that this login did not open (left over from a previous run).
pub fn stale_own_states(
    world: &VoiceWorld,
    bot: UserId,
    session: Option<&str>,
    known: &BTreeSet<(GuildId, ConnectionId)>,
) -> Vec<VState> {
    world
        .of_user(bot)
        .filter(|v| !known.contains(&v.key()) && (v.session.is_none() || v.session.as_deref() != session))
        .cloned()
        .collect()
}
