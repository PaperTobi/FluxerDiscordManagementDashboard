//! Logging in to the web UI with Fluxer (OAuth2 authorization code with PKCE, scope `identify`).

use secrecy::SecretString;
use url::Url;

use super::endpoints::Endpoints;

/// The bot's OAuth2 client (its application id and client secret).
#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: u64,
    pub client_secret: SecretString,
    /// Must match a registered redirect URI exactly.
    pub redirect_uri: Url,
}

/// The person who logged in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthUser {
    pub id: pb_domain::UserId,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

/// Where to send the browser (`state` and the S256 `challenge` come from the caller).
pub fn authorize_url(ep: &Endpoints, client: &OAuthClient, state: &str, challenge: &str) -> String {
    let mut u = Url::parse(&ep.rest("/oauth2/authorize")).unwrap_or_else(|_| ep.api.clone());
    u.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client.client_id.to_string())
        .append_pair("scope", "identify")
        .append_pair("redirect_uri", client.redirect_uri.as_str())
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    u.to_string()
}
