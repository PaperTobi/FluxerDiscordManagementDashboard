//! Where an instance's services are (from `/.well-known/fluxer`).

use pb_domain::UserId;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// The REST origin for bots (`endpoints.api_public`), without `/v1`.
    pub api: Url,
    /// The gateway websocket (`endpoints.gateway`).
    pub gateway: Url,
    /// The media CDN (avatars).
    pub media: Option<Url>,
    /// The instance has voice (an instance without it has nothing for the bot to do).
    pub voice_enabled: bool,
}

fn join(base: &Url, path: &str) -> String {
    format!("{}{path}", base.as_str().trim_end_matches('/'))
}

impl Endpoints {
    /// The gateway URL with the protocol query (`?v=1&encoding=json`).
    pub fn gateway_url(&self) -> Url {
        let mut u = self.gateway.clone();
        if u.path().is_empty() {
            u.set_path("/");
        }
        u.query_pairs_mut()
            .append_pair("v", "1")
            .append_pair("encoding", "json");
        u
    }

    /// A REST URL (`path` starts with `/`, after `/v1`).
    pub fn rest(&self, path: &str) -> String {
        join(&self.api, &format!("/v1{path}"))
    }

    pub fn avatar_url(&self, user: UserId, hash: Option<&str>, size: u32) -> Option<String> {
        let (media, hash) = (self.media.as_ref()?, hash.filter(|h| !h.is_empty())?);
        Some(join(media, &format!("/avatars/{user}/{hash}.webp?size={size}")))
    }

    /// The link that adds the bot to a community with `permissions`.
    pub fn invite_url(&self, application: u64, permissions: u64) -> String {
        join(
            &self.api,
            &format!("/v1/oauth2/authorize?client_id={application}&scope=bot&permissions={permissions}"),
        )
    }
}
