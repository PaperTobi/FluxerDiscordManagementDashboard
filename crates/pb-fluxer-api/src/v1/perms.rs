//! Permission bits and the permission computation (docs: /http-api/permissions/).

use pb_domain::{GuildId, RoleId, UserId};

use super::model::{Channel, OverwriteKind, Role};

pub const ADMINISTRATOR: u64 = 1 << 3;
pub const MANAGE_GUILD: u64 = 1 << 5;
pub const ADD_REACTIONS: u64 = 1 << 6;
pub const VIEW_CHANNEL: u64 = 1 << 10;
pub const SEND_MESSAGES: u64 = 1 << 11;
pub const ATTACH_FILES: u64 = 1 << 15;
pub const READ_MESSAGE_HISTORY: u64 = 1 << 16;
pub const CONNECT: u64 = 1 << 20;
pub const SPEAK: u64 = 1 << 21;
pub const MUTE_MEMBERS: u64 = 1 << 22;
pub const MOVE_MEMBERS: u64 = 1 << 24;
pub const MODERATE_MEMBERS: u64 = 1 << 40;
pub const ALL: u64 = u64::MAX;

/// What the bot needs: see, join and talk in voice; chat for command replies, the no-speak fallback and the mod log
/// (with recordings); reactions to acknowledge commands; history so replies can reference older messages.
pub const BOT: u64 =
    VIEW_CHANNEL | SEND_MESSAGES | CONNECT | SPEAK | ADD_REACTIONS | ATTACH_FILES | READ_MESSAGE_HISTORY;
pub const VOICE: u64 = VIEW_CHANNEL | CONNECT | SPEAK;

/// Stable names of the bits the bot talks about; the message catalog names each as `perm-<name>`.
pub const NAMES: &[(u64, &str)] = &[
    (VIEW_CHANNEL, "view-channel"),
    (SEND_MESSAGES, "send-messages"),
    (ATTACH_FILES, "attach-files"),
    (ADD_REACTIONS, "add-reactions"),
    (READ_MESSAGE_HISTORY, "read-message-history"),
    (CONNECT, "connect"),
    (SPEAK, "speak"),
    (MUTE_MEMBERS, "mute-members"),
    (MOVE_MEMBERS, "move-members"),
    (MODERATE_MEMBERS, "moderate-members"),
    (MANAGE_GUILD, "manage-guild"),
    (ADMINISTRATOR, "administrator"),
];

/// The names of the bits of `need` that `have` lacks.
pub fn missing(have: u64, need: u64) -> Vec<&'static str> {
    NAMES
        .iter()
        .filter(|(bit, _)| need & bit != 0 && have & bit == 0)
        .map(|(_, n)| *n)
        .collect()
}

/// A member's permissions in a community (and channel). `owner`: the community's owner; `roles`: every role of the
/// community; `member_roles`: the member's roles.
pub fn compute(
    guild: GuildId,
    owner: Option<UserId>,
    roles: &[Role],
    user: UserId,
    member_roles: &[RoleId],
    channel: Option<&Channel>,
) -> u64 {
    if owner == Some(user) {
        return ALL;
    }
    let everyone = RoleId(guild.0);
    let mut perms = roles.iter().find(|r| r.id == everyone).map_or(0, |r| r.permissions);
    for r in roles {
        if member_roles.contains(&r.id) {
            perms |= r.permissions;
        }
    }
    if perms & ADMINISTRATOR != 0 {
        return ALL;
    }
    let Some(ch) = channel else { return perms };
    for o in ch
        .overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Role && o.id == guild.0)
    {
        perms = (perms & !o.deny) | o.allow;
    }
    let (mut allow, mut deny) = (0, 0);
    for o in ch
        .overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Role && member_roles.iter().any(|r| r.0 == o.id))
    {
        allow |= o.allow;
        deny |= o.deny;
    }
    perms = (perms & !deny) | allow;
    for o in ch
        .overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Member && o.id == user.0)
    {
        perms = (perms & !o.deny) | o.allow;
    }
    perms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::model::Overwrite;
    use pb_domain::ChannelId;

    fn role(id: u64, p: u64) -> Role {
        Role {
            id: RoleId(id),
            name: String::new(),
            permissions: p,
            position: 0,
        }
    }

    #[test]
    fn owner_admin_roles_and_overwrites() {
        let g = GuildId(1);
        let roles = [
            role(1, VIEW_CHANNEL | SEND_MESSAGES),
            role(2, CONNECT | SPEAK),
            role(3, ADMINISTRATOR),
        ];
        let u = UserId(9);
        assert_eq!(compute(g, Some(u), &roles, u, &[], None), ALL);
        assert_eq!(compute(g, None, &roles, u, &[RoleId(3)], None), ALL);
        assert_eq!(
            compute(g, None, &roles, u, &[RoleId(2)], None),
            VIEW_CHANNEL | SEND_MESSAGES | CONNECT | SPEAK
        );
        let ch = Channel {
            id: ChannelId(5),
            name: "v".into(),
            kind: crate::ChannelKind::Voice,
            parent: None,
            position: 0,
            overwrites: vec![
                Overwrite {
                    id: 1,
                    kind: OverwriteKind::Role,
                    allow: 0,
                    deny: SEND_MESSAGES,
                },
                Overwrite {
                    id: 2,
                    kind: OverwriteKind::Role,
                    allow: SEND_MESSAGES,
                    deny: SPEAK,
                },
                Overwrite {
                    id: 9,
                    kind: OverwriteKind::Member,
                    allow: SPEAK,
                    deny: 0,
                },
            ],
        };
        let p = compute(g, None, &roles, u, &[RoleId(2)], Some(&ch));
        assert_eq!(p, VIEW_CHANNEL | SEND_MESSAGES | CONNECT | SPEAK);
        let other = compute(g, None, &roles, UserId(8), &[RoleId(2)], Some(&ch));
        assert_eq!(other & SPEAK, 0);
        assert_eq!(missing(VIEW_CHANNEL, VOICE), vec!["connect", "speak"]);
    }
}
