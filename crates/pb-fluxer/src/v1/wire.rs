//! Fluxer's JSON → the typed model. Lenient: ids may be strings or numbers, unknown fields are ignored, missing
//! optional fields are defaults. A dispatch the bot cannot read is skipped (and logged), never fatal.

use jiff::Timestamp;
use pb_domain::{ChannelId, ConnectionId, GuildId, MessageId, RoleId, UserId, VoiceState};
use pb_fluxer_api::{
    Channel, ChannelKind, Guild, IncomingMessage, Member, Overwrite, OverwriteKind, Role, User, VoiceGrant,
};
use secrecy::SecretString;
use serde_json::Value;
use url::Url;

/// A u64 from a string or a number.
pub(crate) fn id(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

fn b(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(t)) => t.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Permission masks are decimal strings.
fn mask(v: Option<&Value>) -> u64 {
    id(v).unwrap_or(0)
}

fn ts(v: Option<&Value>) -> Option<Timestamp> {
    v.and_then(Value::as_str).and_then(|t| t.parse().ok())
}

pub(crate) fn user(v: &Value) -> Option<User> {
    Some(User {
        id: UserId(id(v.get("id"))?),
        username: s(v.get("username")).unwrap_or_default(),
        global_name: s(v.get("global_name")),
        avatar: s(v.get("avatar")),
        bot: b(v.get("bot")),
    })
}

pub(crate) fn role(v: &Value) -> Option<Role> {
    Some(Role {
        id: RoleId(id(v.get("id"))?),
        name: s(v.get("name")).unwrap_or_default(),
        permissions: mask(v.get("permissions")),
        position: int(v.get("position")),
    })
}

pub(crate) fn channel(v: &Value) -> Option<Channel> {
    let overwrites = v
        .get("permission_overwrites")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some(Overwrite {
                id: id(o.get("id"))?,
                kind: if int(o.get("type")) == 1 {
                    OverwriteKind::Member
                } else {
                    OverwriteKind::Role
                },
                allow: mask(o.get("allow")),
                deny: mask(o.get("deny")),
            })
        })
        .collect();
    Some(Channel {
        id: ChannelId(id(v.get("id"))?),
        name: s(v.get("name")).unwrap_or_default(),
        kind: ChannelKind::from_code(int(v.get("type"))),
        parent: id(v.get("parent_id")).map(ChannelId),
        position: int(v.get("position")),
        overwrites,
    })
}

pub(crate) fn member(v: &Value) -> Option<Member> {
    let u = v.get("user");
    let uid = id(u.and_then(|u| u.get("id"))).or_else(|| id(v.get("user_id")))?;
    let full = u.filter(|u| u.get("username").is_some()).and_then(user);
    Some(Member {
        user: full,
        id: UserId(uid),
        nick: s(v.get("nick")),
        roles: v
            .get("roles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| id(Some(r)).map(RoleId))
            .collect(),
        mute: b(v.get("mute")),
        deaf: b(v.get("deaf")),
        timed_out_until: ts(v.get("communication_disabled_until")),
    })
}

/// A voice state (`guild` from the payload or the enclosing community). `None` for DM calls and states without a
/// connection (never broadcast anyway).
pub(crate) fn voice_state(v: &Value, guild: Option<GuildId>) -> Option<VoiceState> {
    let guild = id(v.get("guild_id")).map(GuildId).or(guild)?;
    Some(VoiceState {
        guild,
        channel: id(v.get("channel_id")).map(ChannelId),
        user: UserId(id(v.get("user_id"))?),
        connection: ConnectionId(s(v.get("connection_id")).filter(|c| !c.is_empty())?),
        session: s(v.get("session_id")),
        self_mute: b(v.get("self_mute")),
        self_deaf: b(v.get("self_deaf")),
        mute: b(v.get("mute")),
        deaf: b(v.get("deaf")),
        suppress: b(v.get("suppress")),
        e2ee_capable: b(v.get("e2ee_capable")),
        version: u64::try_from(int(v.get("version"))).unwrap_or(0),
    })
}

/// The community's name, icon and owner: in `properties` (GUILD_CREATE) or at the top (GUILD_UPDATE).
pub(crate) fn guild_props(v: &Value) -> (String, Option<String>, Option<UserId>) {
    let p = v.get("properties").filter(|p| p.is_object()).unwrap_or(v);
    let name = s(p.get("name")).or_else(|| s(v.get("name"))).unwrap_or_default();
    let icon = s(p.get("icon")).or_else(|| s(v.get("icon")));
    let owner = id(p.get("owner_id")).or_else(|| id(v.get("owner_id"))).map(UserId);
    (name, icon, owner)
}

pub(crate) fn guild(v: &Value) -> Option<Guild> {
    let gid = GuildId(id(v.get("id"))?);
    let (name, icon, owner) = guild_props(v);
    let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    Some(Guild {
        id: gid,
        name,
        icon,
        owner,
        roles: list("roles").iter().filter_map(role).collect(),
        channels: list("channels").iter().filter_map(channel).collect(),
        members: list("members").iter().filter_map(member).collect(),
        voice_states: list("voice_states")
            .iter()
            .filter_map(|vs| voice_state(vs, Some(gid)))
            .filter(|vs| vs.channel.is_some())
            .collect(),
    })
}

pub(crate) fn message(v: &Value) -> Option<IncomingMessage> {
    let m = v.get("member");
    Some(IncomingMessage {
        id: MessageId(id(v.get("id"))?),
        channel: ChannelId(id(v.get("channel_id"))?),
        guild: id(v.get("guild_id")).map(GuildId),
        author: user(v.get("author")?)?,
        author_roles: m
            .and_then(|m| m.get("roles"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| id(Some(r)).map(RoleId))
            .collect(),
        author_nick: m.and_then(|m| s(m.get("nick"))),
        content: s(v.get("content")).unwrap_or_default(),
        webhook: v.get("webhook_id").is_some_and(|w| !w.is_null()),
        mentions: v
            .get("mentions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|u| id(u.get("id")).map(UserId))
            .collect(),
    })
}

/// The grant's `endpoint` is documented as ws(s)://; add `wss://` only when no scheme is given, map http(s) to ws(s).
pub(crate) fn livekit_endpoint(raw: &str) -> Option<Url> {
    let e = raw.trim();
    if e.is_empty() {
        return None;
    }
    let fixed = match e.split_once("://") {
        Some((scheme, rest)) => match scheme.to_ascii_lowercase().as_str() {
            "https" => format!("wss://{rest}"),
            "http" => format!("ws://{rest}"),
            _ => e.to_owned(),
        },
        None => format!("wss://{e}"),
    };
    Url::parse(&fixed).ok()
}

pub(crate) fn voice_grant(v: &Value) -> Option<VoiceGrant> {
    Some(VoiceGrant::LiveKit {
        guild: GuildId(id(v.get("guild_id"))?),
        channel: ChannelId(id(v.get("channel_id"))?),
        connection: ConnectionId(s(v.get("connection_id"))?),
        endpoint: livekit_endpoint(v.get("endpoint")?.as_str()?)?,
        token: SecretString::from(s(v.get("token"))?),
        e2ee_key: s(v.get("e2ee_key")).filter(|k| !k.is_empty()).map(SecretString::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_guild_create() {
        let g = guild(&json!({
            "id": "1", "unavailable": false,
            "properties": {"name": "Alpha", "owner_id": "9", "icon": null},
            "roles": [{"id": "1", "name": "@everyone", "permissions": "3072", "position": 0}],
            "channels": [{"id": "5", "name": "voice", "type": 2, "permission_overwrites": [{"id": "1", "type": 0, "allow": "0", "deny": "2097152"}]}],
            "members": [{"user": {"id": "7"}, "roles": ["1"], "mute": false}],
            "voice_states": [{"user_id": "7", "channel_id": "5", "connection_id": "abc", "self_mute": true, "version": 3},
                             {"user_id": "8", "channel_id": "5"}]
        }))
        .unwrap();
        assert_eq!((g.id, g.name.as_str(), g.owner), (GuildId(1), "Alpha", Some(UserId(9))));
        assert_eq!(g.roles[0].permissions, 3072);
        assert_eq!(g.channels[0].overwrites[0].deny, 1 << 21);
        assert!(g.members[0].user.is_none(), "an id-only user");
        assert_eq!(
            g.voice_states.len(),
            1,
            "a state without a connection id is not a connection"
        );
        assert_eq!(g.voice_states[0].connection, ConnectionId("abc".into()));
        assert!(g.voice_states[0].self_mute);
    }

    #[test]
    fn reads_messages_grants_and_endpoints() {
        let m = message(&json!({
            "id": "100", "channel_id": "5", "guild_id": "1", "content": "!pb status",
            "author": {"id": "7", "username": "seven", "bot": false},
            "member": {"roles": ["2", "3"], "nick": "Sev"}, "mentions": [{"id": "9"}]
        }))
        .unwrap();
        assert_eq!(m.author_roles, vec![RoleId(2), RoleId(3)]);
        assert_eq!(m.mentions, vec![UserId(9)]);
        assert!(!m.webhook);
        let VoiceGrant::LiveKit { endpoint, e2ee_key, .. } = voice_grant(&json!({
            "token": "jwt", "endpoint": "livekit.example:7880", "connection_id": "c1", "channel_id": "5", "guild_id": "1"
        }))
        .unwrap() else {
            panic!()
        };
        assert_eq!(endpoint.as_str(), "wss://livekit.example:7880/");
        assert!(e2ee_key.is_none());
        assert_eq!(
            livekit_endpoint("https://lk.example/rtc").unwrap().as_str(),
            "wss://lk.example/rtc"
        );
        assert!(
            voice_state(&json!({"user_id": "7", "channel_id": "5", "connection_id": "x"}), None).is_none(),
            "a DM call"
        );
    }
}
