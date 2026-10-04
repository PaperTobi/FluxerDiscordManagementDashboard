//! Fluxer ids. Fluxer snowflakes are 64-bit numbers sent as decimal strings in JSON; they are stored as `u64` and
//! (de)serialised as strings so that JavaScript clients never lose precision.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a Fluxer id: {0:?}")]
pub struct IdError(pub String);

macro_rules! snowflake {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let t = s.trim();
                if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(IdError(s.to_owned()));
                }
                t.parse::<u64>().map($name).map_err(|_| IdError(s.to_owned()))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(untagged)]
                enum Raw<'a> {
                    Str(&'a str),
                    String(String),
                    Num(u64),
                }
                match Raw::deserialize(d)? {
                    Raw::Str(s) => s.parse().map_err(serde::de::Error::custom),
                    Raw::String(s) => s.parse().map_err(serde::de::Error::custom),
                    Raw::Num(n) => Ok($name(n)),
                }
            }
        }
    };
}

snowflake!(
    /// A community ("guild" in the Fluxer API).
    GuildId
);
snowflake!(
    /// A Fluxer account.
    UserId
);
snowflake!(
    /// A text or voice channel.
    ChannelId
);
snowflake!(
    /// A chat message.
    MessageId
);
snowflake!(
    /// A community role.
    RoleId
);

/// The id inside a mention as Fluxer writes it (`<@1>`, `<@!1>`, `<@&2>`, `<#3>`), or the text as it is.
pub fn unmention(s: &str) -> &str {
    let t = s.trim();
    ["<@&", "<@!", "<@", "<#"]
        .iter()
        .find_map(|p| t.strip_prefix(p))
        .and_then(|x| x.strip_suffix('>'))
        .unwrap_or(t)
}

impl UserId {
    /// An id, or a user mention (`<@1>`, `<@!1>`).
    pub fn from_mention(s: &str) -> Option<UserId> {
        let t = s.trim();
        let inner = match t.strip_prefix("<@").and_then(|x| x.strip_suffix('>')) {
            Some(x) if !x.starts_with('&') => x.trim_start_matches('!'),
            Some(_) => return None,
            None => t,
        };
        inner.parse().ok()
    }
}

impl ChannelId {
    /// An id, or a channel mention (`<#1>`).
    pub fn from_mention(s: &str) -> Option<ChannelId> {
        let t = s.trim();
        t.strip_prefix("<#")
            .and_then(|x| x.strip_suffix('>'))
            .unwrap_or(t)
            .parse()
            .ok()
    }
}

/// One voice connection of one account (an account may be connected several times). Opaque string from Fluxer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(pub String);

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_as_strings() {
        let id: UserId = "1234567890123456789".parse().expect("valid id");
        assert_eq!(id.get(), 1_234_567_890_123_456_789);
        let json = serde_json::to_string(&id).expect("serialise");
        assert_eq!(json, "\"1234567890123456789\"");
        assert_eq!(serde_json::from_str::<UserId>(&json).expect("deserialise"), id);
        assert_eq!(serde_json::from_str::<UserId>("42").expect("number form"), UserId(42));
    }

    #[test]
    fn rejects_non_ids() {
        for bad in ["", "abc", "-1", "1.5", "99999999999999999999999"] {
            assert!(bad.parse::<GuildId>().is_err(), "{bad:?} must be rejected");
        }
    }
}
