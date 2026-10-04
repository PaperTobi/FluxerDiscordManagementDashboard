//! Events as the log takes and returns them, and the hash chain: each stored event names the hash of the one before it
//! (the first one names [`GENESIS`]). How a log stores them is its own business (pb-store: one JSON line each).

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// SHA-256 of a stored event (in pb-store: of its line).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineHash(pub [u8; 32]);

/// The `prev` of the first line.
pub const GENESIS: LineHash = LineHash([0; 32]);

impl LineHash {
    pub fn of(line: &[u8]) -> LineHash {
        LineHash(Sha256::digest(line).into())
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for LineHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LineHash({})", &self.hex()[..12])
    }
}

impl fmt::Display for LineHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// Text that is not a SHA-256 hash in hex.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a sha256 hash: {0:?}")]
pub struct NotAHash(pub String);

impl FromStr for LineHash {
    type Err = NotAHash;
    fn from_str(s: &str) -> Result<Self, NotAHash> {
        pb_domain::sha256_from_hex(s)
            .map(LineHash)
            .ok_or_else(|| NotAHash(s.to_owned()))
    }
}

impl Serialize for LineHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for LineHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// An event to append. `ts` is set by the log when `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub kind: String,
    pub v: u32,
    pub ts: Option<Timestamp>,
    pub data: serde_json::Value,
}

/// An event as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub seq: u64,
    pub ts: Timestamp,
    pub kind: String,
    pub v: u32,
    pub prev: LineHash,
    pub data: serde_json::Value,
    /// The hash of this line (the next line's `prev`).
    pub hash: LineHash,
}
