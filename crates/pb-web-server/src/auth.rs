//! Logins: signed cookies, server-side login sessions (`sessions.json`), one-time notices and setup codes.
//!
//! A cookie holds `v1.<id>.<HMAC-SHA256>`; the server keeps records under the SHA-256 of the id, so the file never
//! holds a usable id. Owners stay logged in 12 h, admins 7 d; changing secrets needs a login from the last 15 min.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use http::{HeaderMap, HeaderValue, header};
use jiff::{SignedDuration, Timestamp};
use pb_domain::{GuildId, UserId};
use pb_store_api::{SessionRecord, SessionsFile, StoreError};
use sha2::{Digest, Sha256};

pub const SESSION_COOKIE: &str = "pb_session";
pub const SETUP_COOKIE: &str = "pb_setup";
pub const OAUTH_COOKIE: &str = "pb_oauth";
pub const NOTICE_COOKIE: &str = "pb_notice";

pub const OWNER_LIFETIME: SignedDuration = SignedDuration::from_hours(12);
pub const ADMIN_LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);
pub const FRESH_FOR: SignedDuration = SignedDuration::from_mins(15);
/// `last_seen` is written to the file at most this often (it is kept exactly in memory).
const SEEN_FLUSH: Duration = Duration::from_secs(300);
const NOTICE_LIFETIME: Duration = Duration::from_secs(300);

/// Random bytes, URL-safe base64.
pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    #[expect(
        clippy::expect_used,
        reason = "no secure randomness means no safe way to log anyone in"
    )]
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    B64.encode(b)
}

/// A setup code: 8 characters without look-alikes (no 0/O, 1/I), shown as `XXXX-XXXX`.
pub fn new_setup_code() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut b = [0u8; 8];
    #[expect(clippy::expect_used, reason = "no secure randomness means no safe setup code")]
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    let chars: String = b.iter().map(|x| char::from(ALPHABET[usize::from(x % 32)])).collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// Compares a typed code with the real one, ignoring case, spaces and dashes (in constant time for equal lengths).
pub fn code_matches(typed: &str, code: &str) -> bool {
    let norm = |s: &str| -> Vec<u8> {
        s.bytes()
            .filter(|b| !matches!(b, b'-' | b' ' | b'\t'))
            .map(|b| b.to_ascii_uppercase())
            .collect()
    };
    let (a, b) = (norm(typed), norm(code));
    a.len() == b.len() && a.iter().zip(&b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Signs and checks cookie values and form tokens with the cookie key.
#[derive(Clone)]
pub struct Signer {
    key: Arc<[u8]>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Signer")
    }
}

impl Signer {
    pub fn new(key: &[u8]) -> Signer {
        Signer { key: key.into() }
    }

    fn mac(&self, purpose: &str, data: &str) -> Hmac<Sha256> {
        #[expect(clippy::expect_used, reason = "HMAC takes keys of any length")]
        let mut m = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key length");
        m.update(purpose.as_bytes());
        m.update(b"|");
        m.update(data.as_bytes());
        m
    }

    pub fn tag(&self, purpose: &str, data: &str) -> String {
        B64.encode(self.mac(purpose, data).finalize().into_bytes())
    }

    pub fn check(&self, purpose: &str, data: &str, tag: &str) -> bool {
        B64.decode(tag)
            .is_ok_and(|t| self.mac(purpose, data).verify_slice(&t).is_ok())
    }

    /// `v1.<id>.<tag>`
    pub fn seal(&self, purpose: &str, id: &str) -> String {
        format!("v1.{id}.{}", self.tag(purpose, id))
    }

    /// The id inside a sealed value, if the tag is right.
    pub fn open<'a>(&self, purpose: &str, value: &'a str) -> Option<&'a str> {
        let rest = value.strip_prefix("v1.")?;
        let (id, tag) = rest.rsplit_once('.')?;
        self.check(purpose, id, tag).then_some(id)
    }
}

// ------------------------------------------------------------------------------------------------ cookies

/// A cookie of the request.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// Whether the browser reached us over https (directly or through a proxy that says so).
pub fn https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .next()
                .is_some_and(|p| p.trim().eq_ignore_ascii_case("https"))
        })
}

/// A `Set-Cookie` value (HttpOnly, SameSite=Lax, whole site). `max_age: None` = until the browser closes.
pub fn set_cookie(name: &str, value: &str, max_age: Option<Duration>, secure: bool) -> HeaderValue {
    let mut s = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax");
    if let Some(a) = max_age {
        s.push_str(&format!("; Max-Age={}", a.as_secs()));
    }
    if secure {
        s.push_str("; Secure");
    }
    HeaderValue::from_str(&s).unwrap_or_else(|_| HeaderValue::from_static("pb_invalid=; Max-Age=0"))
}

pub fn clear_cookie(name: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"))
        .unwrap_or_else(|_| HeaderValue::from_static("pb_invalid=; Max-Age=0"))
}

// ------------------------------------------------------------------------------------------------ sessions

/// What a person may see right now (refreshed from Fluxer while they are logged in).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserAccess {
    pub owner: bool,
    pub guilds: BTreeSet<GuildId>,
    /// Changes whenever `owner` or `guilds` change.
    pub epoch: u64,
}

impl UserAccess {
    /// The owner sees every community, admins theirs.
    pub fn may_see(&self, g: GuildId) -> bool {
        self.owner || self.guilds.contains(&g)
    }
}

/// A logged-in session.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    /// The record key (SHA-256 of the cookie's id).
    pub key: String,
    pub record: SessionRecord,
}

struct Inner {
    records: BTreeMap<String, SessionRecord>,
    access: HashMap<UserId, UserAccess>,
    /// Records changed since the last write (besides `last_seen`).
    dirty: bool,
    last_flush: Instant,
}

/// Every login session.
#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Mutex<Inner>>,
    file: Arc<dyn SessionsFile>,
    signer: Signer,
    /// Serializes file writes (the latest state wins).
    writing: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions").finish_non_exhaustive()
    }
}

impl Sessions {
    /// Reads `sessions.json` and drops expired records.
    pub async fn load(file: Arc<dyn SessionsFile>, signer: Signer) -> Result<Sessions, StoreError> {
        let mut records = file.load().await?;
        let now = Timestamp::now();
        let before = records.len();
        records.retain(|_, r| r.expires > now);
        let s = Sessions {
            inner: Arc::new(Mutex::new(Inner {
                dirty: records.len() != before,
                records,
                access: HashMap::new(),
                last_flush: Instant::now(),
            })),
            file,
            signer,
            writing: Arc::new(tokio::sync::Mutex::new(())),
        };
        let users: Vec<(UserId, bool)> = s.lock().records.values().map(|r| (r.user, r.owner)).collect();
        for (u, owner) in users {
            s.lock().access.entry(u).or_insert(UserAccess {
                owner,
                ..UserAccess::default()
            });
        }
        Ok(s)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn signer(&self) -> &Signer {
        &self.signer
    }

    /// Starts a session; returns the cookie value.
    pub async fn create(&self, record: SessionRecord, access: UserAccess) -> String {
        let id = random_token(32);
        let key = sha256_hex(&id);
        {
            let mut g = self.lock();
            let epoch = g.access.get(&record.user).map_or(0, |a| a.epoch + 1);
            g.access.insert(record.user, UserAccess { epoch, ..access });
            g.records.insert(key, record);
            g.dirty = true;
        }
        self.flush().await;
        self.signer.seal("session", &id)
    }

    /// The session a request carries (valid and not expired); notes that it was seen.
    pub fn lookup(&self, headers: &HeaderMap) -> Option<ActiveSession> {
        let id = self.signer.open("session", cookie(headers, SESSION_COOKIE)?)?;
        let key = sha256_hex(id);
        let now = Timestamp::now();
        let mut g = self.lock();
        let r = g.records.get_mut(&key)?;
        if r.expires <= now {
            return None;
        }
        r.last_seen = now;
        Some(ActiveSession { key, record: r.clone() })
    }

    /// The record under `key`, if it is still valid.
    pub fn get(&self, key: &str) -> Option<SessionRecord> {
        let g = self.lock();
        g.records.get(key).filter(|r| r.expires > Timestamp::now()).cloned()
    }

    pub async fn remove(&self, key: &str) {
        let removed = {
            let mut g = self.lock();
            let r = g.records.remove(key).is_some();
            g.dirty |= r;
            r
        };
        if removed {
            self.flush().await;
        }
    }

    /// Ends every session of a person.
    pub async fn remove_user(&self, user: UserId) {
        {
            let mut g = self.lock();
            let before = g.records.len();
            g.records.retain(|_, r| r.user != user);
            g.dirty |= g.records.len() != before;
        }
        self.flush().await;
    }

    /// The form token of a session.
    pub fn csrf(&self, key: &str) -> String {
        self.signer.tag("csrf", key)
    }

    pub fn csrf_ok(&self, key: &str, token: &str) -> bool {
        self.signer.check("csrf", key, token)
    }

    pub fn access(&self, user: UserId) -> UserAccess {
        self.lock().access.get(&user).cloned().unwrap_or_default()
    }

    /// Sets what a person may see (the epoch moves when it changed).
    pub fn set_access(&self, user: UserId, owner: bool, guilds: BTreeSet<GuildId>) {
        let mut g = self.lock();
        let a = g.access.entry(user).or_default();
        if a.owner != owner || a.guilds != guilds {
            a.owner = owner;
            a.guilds = guilds;
            a.epoch += 1;
        }
    }

    /// People with a valid session.
    pub fn users(&self) -> BTreeSet<UserId> {
        let now = Timestamp::now();
        self.lock()
            .records
            .values()
            .filter(|r| r.expires > now)
            .map(|r| r.user)
            .collect()
    }

    /// Drops expired sessions and writes the file when something changed (or `last_seen` is due).
    pub async fn flush(&self) {
        let _w = self.writing.lock().await;
        let snapshot = {
            let mut g = self.lock();
            let now = Timestamp::now();
            let before = g.records.len();
            g.records.retain(|_, r| r.expires > now);
            g.dirty |= g.records.len() != before;
            let due = g.last_flush.elapsed() >= SEEN_FLUSH;
            if !(g.dirty || due) {
                return;
            }
            g.dirty = false;
            g.last_flush = Instant::now();
            let users: BTreeSet<UserId> = g.records.values().map(|r| r.user).collect();
            g.access.retain(|u, _| users.contains(u));
            g.records.clone()
        };
        if let Err(e) = self.file.save(&snapshot).await {
            tracing::error!(error = %e, "login sessions could not be saved");
            self.lock().dirty = true;
        }
    }
}

/// A new session record.
pub fn record(user: UserId, name: String, avatar: Option<String>, owner: bool) -> SessionRecord {
    let now = Timestamp::now();
    let life = if owner { OWNER_LIFETIME } else { ADMIN_LIFETIME };
    SessionRecord {
        user,
        name,
        avatar,
        owner,
        created: now,
        expires: now.saturating_add(life).unwrap_or(now),
        fresh_until: now.saturating_add(FRESH_FOR).unwrap_or(now),
        last_seen: now,
    }
}

// ------------------------------------------------------------------------------------------------ notices

/// Messages for the next page a browser opens (after a form or a login), kept for a few minutes.
#[derive(Clone, Default)]
pub struct Notices {
    inner: Arc<Mutex<HashMap<String, (pb_web::app::Notice, Instant)>>>,
}

impl std::fmt::Debug for Notices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notices")
    }
}

impl Notices {
    /// Stores a notice; returns the `Set-Cookie` value that points the browser at it.
    pub fn put(&self, ok: bool, text: String, secure: bool) -> HeaderValue {
        self.put_fields(ok, text, Vec::new(), secure)
    }

    /// A notice that also says what happened to each field (shown next to it).
    pub fn put_fields(&self, ok: bool, text: String, fields: Vec<pb_web::app::FieldNote>, secure: bool) -> HeaderValue {
        self.store(
            pb_web::app::Notice {
                ok,
                text,
                log_in_again: None,
                fields,
            },
            secure,
        )
    }

    /// A notice with a link to log in again and come back to `next`.
    pub fn put_log_in_again(&self, text: String, next: String, secure: bool) -> HeaderValue {
        self.store(
            pb_web::app::Notice {
                ok: false,
                text,
                log_in_again: Some(next),
                fields: Vec::new(),
            },
            secure,
        )
    }

    fn store(&self, notice: pb_web::app::Notice, secure: bool) -> HeaderValue {
        let id = random_token(16);
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retain(|_, (_, at)| at.elapsed() < NOTICE_LIFETIME);
        g.insert(id.clone(), (notice, Instant::now()));
        set_cookie(NOTICE_COOKIE, &id, Some(NOTICE_LIFETIME), secure)
    }

    pub fn take(&self, headers: &HeaderMap) -> Option<pb_web::app::Notice> {
        let id = cookie(headers, NOTICE_COOKIE)?;
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.remove(id)
            .filter(|(_, at)| at.elapsed() < NOTICE_LIFETIME)
            .map(|(n, _)| n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_codes() {
        let c = new_setup_code();
        assert_eq!(c.len(), 9);
        assert!(!c.contains(['0', 'O', '1', 'I']));
        assert!(code_matches(&c.to_lowercase().replace('-', " "), &c));
        assert!(!code_matches("AAAA-AAAA", "AAAA-AAAB"));
        assert!(!code_matches("AAAA", "AAAA-AAAA"));
    }

    #[test]
    fn sealed_values() {
        let s = Signer::new(b"key");
        let v = s.seal("session", "abc");
        assert_eq!(s.open("session", &v), Some("abc"));
        assert_eq!(s.open("setup", &v), None, "another purpose");
        assert_eq!(Signer::new(b"other").open("session", &v), None, "another key");
        assert_eq!(s.open("session", &v.replace("abc", "abd")), None, "a changed id");
    }

    #[test]
    fn cookies_are_found() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; pb_session=v1.x.y; b=2"));
        assert_eq!(cookie(&h, SESSION_COOKIE), Some("v1.x.y"));
        assert_eq!(cookie(&h, "c"), None);
    }
}
