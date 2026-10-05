//! What the pages learn about a request (who is logged in, notices, the setup wizard).

use axum::extract::ConnectInfo;
use http::request::Parts;
use pb_web::app::{Host, Notice, SetupView, Viewer};

use super::server::WebState;
use super::util::locale_of;

pub(crate) struct PageHost {
    pub st: WebState,
}

impl Host for PageHost {
    fn viewer(&self, parts: &Parts) -> Option<Viewer> {
        let s = self.st.sessions.lookup(&parts.headers)?;
        let access = self.st.sessions.access(s.record.user);
        let avatar = self
            .st
            .engine
            .endpoints()
            .and_then(|ep| ep.avatar_url(s.record.user, s.record.avatar.as_deref(), 64));
        Some(Viewer {
            user: s.record.user,
            name: s.record.name.clone(),
            avatar,
            owner: access.owner,
            guilds: access.guilds,
            locale: locale_of(&parts.headers),
            csrf: self.st.sessions.csrf(&s.key),
        })
    }

    fn take_notice(&self, parts: &Parts) -> Option<Notice> {
        self.st.notices.take(&parts.headers)
    }

    fn invite_url(&self, permissions: u64) -> Option<String> {
        let client = self.st.secrets.client_id()?;
        Some(self.st.engine.endpoints()?.invite_url(client, permissions))
    }

    fn api_tokens(&self) -> Vec<pb_web::app::ApiTokenView> {
        self.st
            .api
            .tokens()
            .into_iter()
            .map(|t| pb_web::app::ApiTokenView {
                id: t.id,
                name: t.name,
                scopes: t.scopes,
                communities: t.communities,
                created_ms: t.created.as_millisecond(),
                last_used_ms: t.last_used.map(|t| t.as_millisecond()),
            })
            .collect()
    }

    fn secrets_from_env(&self) -> (bool, bool) {
        (self.st.cfg.token_from_env, self.st.cfg.client_secret_from_env)
    }

    fn repairing(&self) -> bool {
        self.st.repairing()
    }

    fn setup(&self, parts: &Parts) -> SetupView {
        let ip = parts
            .extensions
            .get::<ConnectInfo<super::tls::Peer>>()
            .map(|c| c.0.0.ip());
        self.st.setup_view(&parts.headers, ip)
    }
}
