//! Audio for the pages: library clips, sentence recordings (with the community's access rules) and voice-line
//! previews rendered on request. Files are served with range requests (seeking works).

use axum::extract::{Path, Query, Request, State};
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use pb_domain::{BlobHash, Scope, SentenceId};
use pb_voicelines::LineKey;
use serde::Deserialize;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use super::forms::parse_scope;
use super::server::WebState;

async fn serve_blob(st: &WebState, hash: &BlobHash, req: Request) -> Response {
    let Some(path) = st.blobs.path(hash).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match ServeFile::new(path).oneshot(req).await {
        Ok(mut r) => {
            // Blobs are stored without a file name: every audio blob the pages ask for is WAV.
            r.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
            r.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            );
            r.into_response()
        }
        Err(never) => match never {},
    }
}

/// `GET /media/clip/<hash>`: a library clip's prepared render (or a shipped clip).
pub async fn clip(State(st): State<WebState>, Path(hash): Path<String>, req: Request) -> Response {
    if st.sessions.lookup(req.headers()).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(h) = hash.parse::<BlobHash>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if st.engine.clip(&h).is_none() && !st.engine.shipped_clip(&h) {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_blob(&st, &h, req).await
}

/// `GET /media/sentence/<id>`: a sentence's recording, for the bot owner and (when allowed) the community's admins.
pub async fn sentence(State(st): State<WebState>, Path(id): Path<String>, req: Request) -> Response {
    let Some(login) = st.sessions.lookup(req.headers()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(id) = id.parse::<SentenceId>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Some(row)) = st.index.sentence(id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let access = st.sessions.access(login.record.user);
    let g = row.record.guild;
    let eff = st.engine.settings().current().effective(Some(g), None);
    if !eff.may_play_recordings(access.owner, access.may_see(g)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match row.record.audio.filter(|_| !row.audio_deleted) {
        Some(h) => serve_blob(&st, &h, req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    scope: String,
    line: String,
    /// Said in this language (empty: as the bot would choose there now).
    #[serde(default)]
    lang: String,
}

/// `GET /media/preview?scope=…&line=…[&lang=…]`: a voice line as the bot would say it there now, or in a language.
pub async fn preview(State(st): State<WebState>, headers: HeaderMap, Query(q): Query<PreviewQuery>) -> Response {
    let Some(login) = st.sessions.lookup(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let access = st.sessions.access(login.record.user);
    let (Some(scope), Ok(key)) = (parse_scope(&q.scope), q.line.parse::<LineKey>()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The community it is heard in: the scope's, or for global lines the first one this login may see.
    let (guild, person) = match scope {
        Scope::Global => (
            st.engine.guilds().available().into_iter().find(|g| access.may_see(*g)),
            None,
        ),
        Scope::Server { guild } => (Some(guild), None),
        Scope::Person { guild, user } => (Some(guild), Some(user)),
    };
    let Some(guild) = guild.filter(|g| access.may_see(*g)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let lang = match q.lang.trim() {
        "" => None,
        l => match l.parse::<pb_domain::Lang>() {
            Ok(l) => Some(l),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    match st.engine.preview(guild, person, key.0, lang).await {
        Ok(r) => (
            [(header::CONTENT_TYPE, "audio/wav"), (header::CACHE_CONTROL, "no-store")],
            r.wav(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            pb_web::fmt::engine_error(super::util::locale_of(&headers), &e),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct MembersQuery {
    guild: u64,
    #[serde(default)]
    q: String,
}

/// `GET /api/members?guild=…&q=…`: people in a community whose name starts with `q` (for the "track someone" box).
pub async fn members(State(st): State<WebState>, headers: HeaderMap, Query(q): Query<MembersQuery>) -> Response {
    let Some(login) = st.sessions.lookup(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let access = st.sessions.access(login.record.user);
    let g = pb_domain::GuildId(q.guild);
    if !access.may_see(g) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let query = q.q.trim();
    if query.is_empty() {
        return axum::Json(Vec::<pb_live_proto::Who>::new()).into_response();
    }
    match st.engine.search_members(g, query, 20).await {
        Ok(found) => axum::Json(found).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            pb_web::fmt::engine_error(super::util::locale_of(&headers), &e),
        )
            .into_response(),
    }
}
