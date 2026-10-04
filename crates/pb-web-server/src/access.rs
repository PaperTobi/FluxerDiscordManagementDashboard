//! What a live connection may see, from its login; kept current from Fluxer while people are logged in.

use pb_domain::GuildId;

use super::auth::Sessions;
use super::login::is_owner;
use super::server::WebState;

/// A live connection's view of its login.
pub(crate) struct LoginAccess {
    pub sessions: Sessions,
    /// The session record's key.
    pub key: String,
}

impl pb_live::Access for LoginAccess {
    fn owner(&self) -> bool {
        self.sessions
            .get(&self.key)
            .is_some_and(|r| self.sessions.access(r.user).owner)
    }

    fn guild(&self, g: GuildId) -> bool {
        self.sessions
            .get(&self.key)
            .is_some_and(|r| self.sessions.access(r.user).guilds.contains(&g))
    }

    fn expired(&self) -> bool {
        self.sessions.get(&self.key).is_none()
    }

    fn epoch(&self) -> u64 {
        self.sessions
            .get(&self.key)
            .map_or(0, |r| self.sessions.access(r.user).epoch)
    }
}

/// Reads again who is an owner and which communities each logged-in person administers. Communities the bot cannot see
/// right now (not connected, a community unavailable) keep the last known answer: nobody loses access because Fluxer
/// is unreachable.
pub(crate) async fn refresh(st: &WebState) {
    let connected = matches!(st.engine.connection(), pb_engine::Connection::Ready);
    for user in st.sessions.users() {
        let owner = is_owner(st, user);
        let guilds = if owner {
            Default::default()
        } else if connected {
            let known: std::collections::BTreeSet<GuildId> = st.engine.guilds().available().into_iter().collect();
            let mut g = st.engine.admin_guilds(user).await;
            g.extend(
                st.sessions
                    .access(user)
                    .guilds
                    .into_iter()
                    .filter(|x| !known.contains(x)),
            );
            g
        } else {
            st.sessions.access(user).guilds
        };
        st.sessions.set_access(user, owner, guilds);
    }
}
