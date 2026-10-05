//! Assembly: the shared state, the routes, the listener and the background upkeep.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{FromRef, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use leptos::prelude::*;
use leptos_axum::{LeptosRoutes, generate_route_list};
use pb_engine::Engine;
use pb_live::SessionCfg;
use pb_store_api::{BlobStore, EventLog, Index, Secrets, SecretsFile, SessionsFile, StoreError};
use pb_web::app::{App, AppCtx, shell};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::watch;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use super::auth::{Notices, Sessions, Signer, new_setup_code, random_token};
use super::login::PendingLogin;
use super::setup::Wizard;

/// How the web server runs.
#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
    /// Holds `pkg/`: the browser bundle (`pb.js`, `pb_bg.wasm`) and the stylesheet (`pb.css`).
    pub site_root: PathBuf,
    /// Live connection timings.
    pub live: SessionCfg,
    /// Where the setup code is written (shown on the setup page).
    pub setup_code_file: String,
    /// The bot token comes from the environment (it cannot be replaced in the web UI).
    pub token_from_env: bool,
    /// The client secret comes from the environment.
    pub client_secret_from_env: bool,
    /// Serve HTTPS with this configuration (`None`: plain HTTP, e.g. behind a reverse proxy).
    pub tls: Option<Arc<tokio_rustls::rustls::ServerConfig>>,
}

/// What the web server runs on.
#[derive(Clone)]
pub struct WebParts {
    pub engine: Arc<Engine>,
    pub index: Arc<dyn Index>,
    pub blobs: Arc<dyn BlobStore>,
    pub log: Arc<dyn EventLog>,
    pub secrets: Arc<dyn SecretsFile>,
    pub sessions: Arc<dyn SessionsFile>,
    /// The read API, served at `/api/v1`.
    pub api: Arc<pb_api::Api>,
    pub version: String,
    /// Becomes `true` when the bot stops.
    pub shutdown: watch::Receiver<bool>,
}

/// `secrets.toml` as last read or written (writes go through here so readers never wait for the disk).
pub(crate) struct SecretsCache {
    pub file: Arc<dyn SecretsFile>,
    current: RwLock<Secrets>,
    write: tokio::sync::Mutex<()>,
}

impl SecretsCache {
    pub fn get(&self) -> Secrets {
        self.current.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Changes the secrets and writes the file (nothing changes when the write fails).
    pub async fn update(&self, f: impl FnOnce(&mut Secrets)) -> Result<(), StoreError> {
        let _w = self.write.lock().await;
        let mut s = self.get();
        f(&mut s);
        self.file.save(&s).await?;
        if let Ok(mut c) = self.current.write() {
            *c = s;
        }
        Ok(())
    }

    /// The application (OAuth2 client) id: the part of the bot token before the dot.
    pub fn client_id(&self) -> Option<u64> {
        let s = self.get();
        let token = s.bot_token?;
        token
            .expose_secret()
            .split_once('.')
            .and_then(|(id, _)| id.parse().ok())
    }
}

impl std::fmt::Debug for WebParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebParts")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// Everything the handlers share.
#[derive(Clone)]
pub struct WebState {
    pub engine: Arc<Engine>,
    pub index: Arc<dyn Index>,
    pub blobs: Arc<dyn BlobStore>,
    pub log: Arc<dyn EventLog>,
    pub sessions: Sessions,
    pub notices: Notices,
    pub(crate) secrets: Arc<SecretsCache>,
    pub(crate) wizard: Arc<Mutex<Wizard>>,
    pub api: Arc<pb_api::Api>,
    /// Fluxer refused the saved client secret at the last login (it was reset, or is wrong).
    pub(crate) client_rejected: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) logins: Arc<Mutex<HashMap<String, PendingLogin>>>,
    pub cfg: Arc<WebConfig>,
    pub version: String,
    pub(crate) leptos: LeptosOptions,
    pub(crate) shutdown: watch::Receiver<bool>,
}

impl std::fmt::Debug for WebState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebState")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl FromRef<WebState> for LeptosOptions {
    fn from_ref(s: &WebState) -> Self {
        s.leptos.clone()
    }
}

impl WebState {
    /// Reads secrets and sessions; creates the cookie key at the first start and a setup code while setup is unfinished.
    pub async fn new(cfg: WebConfig, parts: WebParts) -> Result<WebState, StoreError> {
        let secrets = Arc::new(SecretsCache {
            current: RwLock::new(parts.secrets.load().await?),
            file: parts.secrets,
            write: tokio::sync::Mutex::new(()),
        });
        let key = match secrets.get().cookie_key {
            Some(k) => k.expose_secret().to_owned(),
            None => {
                let k = random_token(32);
                secrets
                    .update(|s| s.cookie_key = Some(SecretString::from(k.clone())))
                    .await?;
                k
            }
        };
        let signer = Signer::new(key.as_bytes());
        let sessions = Sessions::load(parts.sessions, signer).await?;
        let mut wizard = Wizard::default();
        if secrets.get().setup.done {
            secrets.file.write_setup_code(None).await?;
        } else {
            let code = new_setup_code();
            secrets.file.write_setup_code(Some(&code)).await?;
            tracing::warn!(
                "setup is not finished: open the web UI and enter the setup code {code} (it is also in {})",
                cfg.setup_code_file
            );
            wizard.code = Some(code);
        }
        let leptos = LeptosOptions::builder()
            .output_name("pb")
            .site_root(cfg.site_root.to_string_lossy().into_owned())
            .site_pkg_dir("pkg")
            .site_addr(cfg.bind)
            .hash_files(false)
            .build();
        Ok(WebState {
            engine: parts.engine,
            index: parts.index,
            blobs: parts.blobs,
            log: parts.log,
            sessions,
            notices: Notices::default(),
            secrets,
            wizard: Arc::new(Mutex::new(wizard)),
            api: parts.api,
            client_rejected: Arc::default(),
            logins: Arc::new(Mutex::new(HashMap::new())),
            cfg: Arc::new(cfg),
            version: parts.version,
            leptos,
            shutdown: parts.shutdown,
        })
    }

    /// What the pages get.
    pub(crate) fn app_ctx(&self) -> AppCtx {
        AppCtx {
            engine: self.engine.clone(),
            index: self.index.clone(),
            blobs: self.blobs.clone(),
            log: self.log.clone(),
            version: self.version.clone(),
            host: Arc::new(super::host::PageHost { st: self.clone() }),
        }
    }
}

/// Renders a page (the router decides which; unknown paths get the "not found" page with status 404).
fn render(st: &WebState) -> impl Fn(Request<Body>) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> + Clone {
    let ctx = st.app_ctx();
    let opts = st.leptos.clone();
    leptos_axum::render_app_to_stream_with_context(move || provide_context(ctx.clone()), move || shell(opts.clone()))
}

/// For container health checks: the process serves and no model thread hangs (503 then: a restart helps). The Fluxer
/// connection is reported but does not make it unhealthy (a missing or rejected token is fixed in the web UI, not by a
/// restart).
/// `GET /healthz`: 503 when a model thread hangs or a part of the engine failed or stopped answering (the service
/// manager restarts the bot); "degraded" while a crashed part is being started again.
async fn healthz(axum::extract::State(st): axum::extract::State<WebState>) -> Response {
    let fluxer = format!("{:?}", st.engine.connection());
    let health = st.engine.health();
    let stuck = st.engine.stuck_model();
    let parts: Vec<_> = health
        .actors
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "state": format!("{:?}", a.state),
                "restarts": a.restarts,
                "waiting": a.queued,
                "error": a.last_error,
            })
        })
        .collect();
    let (code, status) = if stuck.is_some() || health.failing() {
        (StatusCode::SERVICE_UNAVAILABLE, "failing")
    } else if health.degraded() {
        (StatusCode::OK, "degraded")
    } else {
        (StatusCode::OK, "ok")
    };
    let body = serde_json::json!({
        "status": status,
        "version": st.version,
        "fluxer": fluxer,
        "stuck_model": stuck.map(|m| m.name()),
        "fatal": health.fatal.map(|f| f.to_string()),
        "parts": parts,
    });
    (code, axum::Json(body)).into_response()
}

async fn not_found(axum::extract::State(st): axum::extract::State<WebState>, req: Request<Body>) -> Response {
    let mut res = render(&st)(req).await;
    *res.status_mut() = StatusCode::NOT_FOUND;
    res
}

/// Every route.
pub fn router(st: WebState) -> Router {
    let routes = generate_route_list(App);
    let ctx = st.app_ctx();
    let opts = st.leptos.clone();
    let pkg = ServeDir::new(st.cfg.site_root.join("pkg"))
        .precompressed_br()
        .precompressed_gzip();
    Router::new()
        .route("/live", get(super::live::live))
        .route("/healthz", get(healthz))
        .route("/login", get(super::login::start))
        .route("/auth/callback", get(super::login::callback))
        .route("/auth/logout", post(super::login::logout))
        .route("/setup", post(super::setup::submit))
        .route("/settings", post(super::forms::settings))
        .route("/settings/reset", post(super::forms::reset))
        .route("/settings/list", post(super::forms::list))
        .route("/people/track", post(super::forms::track))
        .route("/people/untrack", post(super::forms::untrack))
        .route("/jar/reset", post(super::forms::jar_reset))
        .route("/community/resume-joining", post(super::forms::resume_joining))
        .route("/say", post(super::forms::say))
        .route("/reports/send", post(super::forms::send_report))
        .route("/evidence/delete", post(super::forms::delete_recording))
        .route("/voice-lines", post(super::voice::edit))
        .route(
            "/clips",
            post(super::voice::upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/clips/update", post(super::voice::update))
        .route("/clips/remove", post(super::voice::remove))
        .route("/system/token", post(super::system::token))
        .route("/system/client-secret", post(super::system::client_secret))
        .route("/system/reconnect", post(super::system::reconnect))
        .route("/system/reload", post(super::system::reload))
        .route("/system/retry-log", post(super::system::retry_log))
        .route("/system/api/create", post(super::system::api_create))
        .route("/system/api/revoke", post(super::system::api_revoke))
        .route("/media/clip/{hash}", get(super::media::clip))
        .route("/media/sentence/{id}", get(super::media::sentence))
        .route("/media/preview", get(super::media::preview))
        .route("/api/members", get(super::media::members))
        .nest_service(pb_api_proto::v1::BASE, st.api.router())
        .nest(
            "/pkg",
            Router::new()
                .fallback_service(pkg)
                .layer(SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("no-cache"),
                )),
        )
        .leptos_routes_with_context(
            &st,
            routes,
            move || provide_context(ctx.clone()),
            move || shell(opts.clone()),
        )
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(st.clone(), super::hosts::guard))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .with_state(st)
}

/// Serves until `shutdown` turns `true`. Also keeps logins up to date: who may see which community (every 30 s),
/// expired sessions and the sessions file.
pub async fn serve(st: WebState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    let upkeep = tokio::spawn(upkeep(st.clone()));
    let repair = tokio::spawn(super::setup::watch_credentials(st.clone()));
    let mut stop = st.shutdown.clone();
    let app = router(st.clone());
    let stopped = async move {
        let _ = stop.wait_for(|s| *s).await;
    };
    let res = match st.cfg.tls.clone() {
        None => {
            axum::serve(listener, app.into_make_service_with_connect_info::<super::tls::Peer>())
                .with_graceful_shutdown(stopped)
                .await
        }
        Some(tls) => {
            // Everything that asks whether a request came over HTTPS (secure cookies, origins) reads this header.
            let app = app.layer(axum::middleware::map_request(|mut req: Request<Body>| async move {
                req.headers_mut()
                    .insert("x-forwarded-proto", HeaderValue::from_static("https"));
                req
            }));
            axum::serve(
                super::tls::TlsListener::new(listener, tls),
                app.into_make_service_with_connect_info::<super::tls::Peer>(),
            )
            .with_graceful_shutdown(stopped)
            .await
        }
    };
    upkeep.abort();
    repair.abort();
    st.sessions.flush().await;
    res
}

async fn upkeep(st: WebState) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        super::access::refresh(&st).await;
        st.sessions.flush().await;
        super::login::forget_stale(&st);
    }
}

pub(crate) fn redirect_with(to: &str, cookies: Vec<HeaderValue>) -> Response {
    let mut res = axum::response::Redirect::to(to).into_response();
    for c in cookies {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}
