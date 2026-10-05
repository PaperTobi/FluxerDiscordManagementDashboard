//! The page shell, the layout (sidebar and main area) and the routes.

use std::collections::BTreeSet;
use std::sync::Arc;

use leptos::prelude::*;
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::{ParamSegment, StaticSegment};
use pb_domain::{GuildId, UserId};
use pb_i18n::{Locale, text};
use pb_live_proto::{Topic, TopicState};

use crate::islands::SidebarLive;
use crate::pages;

/// What the server gives every page.
#[derive(Clone)]
pub struct AppCtx {
    pub engine: Arc<pb_engine::Engine>,
    pub index: Arc<dyn pb_store_api::Index>,
    pub blobs: Arc<dyn pb_store_api::BlobStore>,
    pub log: Arc<dyn pb_store_api::EventLog>,
    pub version: String,
    pub host: Arc<dyn Host>,
}

impl std::fmt::Debug for AppCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCtx")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// What the web server knows about a request.
pub trait Host: Send + Sync {
    /// Who is logged in (`None` = nobody).
    fn viewer(&self, parts: &http::request::Parts) -> Option<Viewer>;
    /// The message the last form or login left for this browser (shown once).
    fn take_notice(&self, parts: &http::request::Parts) -> Option<Notice>;
    /// The setup wizard as this browser sees it.
    fn setup(&self, parts: &http::request::Parts) -> SetupView;
    /// Fluxer no longer accepts the bot token or client secret (the setup page replaces them).
    fn repairing(&self) -> bool;
    /// The link that invites the bot to a community with these permissions (`None` until the bot token is known and
    /// the bot has found its instance).
    fn invite_url(&self, permissions: u64) -> Option<String>;
    /// Which secrets come from the bot's environment (bot token, client secret): they cannot be replaced here.
    fn secrets_from_env(&self) -> (bool, bool);
}

/// A message shown once at the top of the next page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub ok: bool,
    pub text: String,
    /// Offer to log in again and come back to this path.
    pub log_in_again: Option<String>,
    /// What happened to each field of the form, shown next to it.
    pub fields: Vec<FieldNote>,
}

/// What happened to one field of a form (a setting), shown next to it on the next page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldNote {
    /// The field: a setting's key.
    pub key: String,
    pub ok: bool,
    pub text: String,
    /// What was typed, when it was refused (the field shows it again instead of the old value).
    pub typed: Option<String>,
}

/// This request's notice (taken from the store once, then shared by the notice bar and the forms).
#[derive(Debug, Clone)]
struct CurrentNotice(Option<Notice>);

/// The notice the last form left for this page (taken from the store once per request).
pub fn current_notice() -> Option<Notice> {
    if let Some(n) = use_context::<CurrentNotice>() {
        return n.0;
    }
    let n = use_context::<http::request::Parts>().and_then(|p| app().host.take_notice(&p));
    provide_context(CurrentNotice(n.clone()));
    n
}

/// What the notice says about one field.
pub fn field_note(key: &str) -> Option<FieldNote> {
    current_notice().and_then(|n| n.fields.into_iter().find(|f| f.key == key))
}

/// Where the setup wizard is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SetupStep {
    /// Enter the setup code (it proves access to the server).
    Code,
    /// Which Fluxer instance.
    Instance,
    /// The bot token.
    Token,
    /// The client secret (and the redirect URI to register).
    ClientSecret,
    /// Log in with Fluxer to become the bot owner.
    Owner,
    Done,
}

/// The setup wizard for one browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupView {
    /// The step shown.
    pub step: SetupStep,
    /// The furthest step reached (the steps before it are done and can be opened again).
    pub reached: SetupStep,
    /// The token the wizard's forms carry.
    pub csrf: String,
    /// The Fluxer instance (API address).
    pub instance: String,
    /// What was last typed for the instance when it did not work.
    pub typed_instance: Option<String>,
    /// The bot's name once its token works.
    pub bot: Option<String>,
    /// The address Fluxer sends people back to after logging in (register it with the application).
    pub redirect_uri: String,
    /// Seconds before this browser may try another code.
    pub wait_secs: u64,
    /// Where the operator finds the setup code.
    pub code_file: String,
    /// The client secret comes from the environment (it cannot be changed here).
    pub secret_from_env: bool,
    /// A bot token is saved.
    pub has_token: bool,
    /// A client secret is saved.
    pub has_secret: bool,
    /// Setup is finished, but Fluxer no longer accepts the bot token or client secret: they are replaced here.
    pub repair: bool,
}

/// The person looking at the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub user: UserId,
    pub name: String,
    pub avatar: Option<String>,
    /// The bot owner (or an extra bot owner): sees everything.
    pub owner: bool,
    /// Communities this person administers.
    pub guilds: BTreeSet<GuildId>,
    pub locale: Locale,
    /// The token forms must carry.
    pub csrf: String,
}

impl Viewer {
    pub fn may_see(&self, g: GuildId) -> bool {
        self.owner || self.guilds.contains(&g)
    }

    /// Whether this login may play the community's recordings.
    #[cfg(feature = "ssr")]
    pub fn may_play_recordings(&self, g: GuildId) -> bool {
        app()
            .engine
            .settings()
            .current()
            .effective(Some(g), None)
            .may_play_recordings(self.owner, self.guilds.contains(&g))
    }
}

pub fn app() -> AppCtx {
    expect_context::<AppCtx>()
}

/// The viewer of this request (pages are only rendered for logged-in people; see [`App`]).
pub fn viewer() -> Option<Viewer> {
    let parts = use_context::<http::request::Parts>()?;
    app().host.viewer(&parts)
}

pub fn locale() -> Locale {
    viewer()
        .map(|v| v.locale)
        .or_else(|| {
            use_context::<http::request::Parts>().and_then(|p| {
                p.headers
                    .get("accept-language")
                    .and_then(|h| h.to_str().ok())
                    .map(Locale::negotiate)
            })
        })
        .unwrap_or(Locale::En)
}

pub fn t(id: &str) -> String {
    text(locale(), id, &[])
}

/// The page's security policy: scripts only from this site or with this response's nonce (the hydration script),
/// WebAssembly allowed, styles only from the stylesheet, pictures also from Fluxer's media server, no framing.
fn security_policy() {
    let (Some(nonce), Some(res)) = (
        leptos::nonce::use_nonce(),
        use_context::<leptos_axum::ResponseOptions>(),
    ) else {
        return;
    };
    let policy = format!(
        "default-src 'self'; script-src 'self' 'nonce-{nonce}' 'wasm-unsafe-eval'; style-src 'self'; \
         img-src 'self' https: data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; \
         base-uri 'none'; form-action 'self'; object-src 'none'"
    );
    if let Ok(v) = http::HeaderValue::from_str(&policy) {
        res.insert_header(http::header::CONTENT_SECURITY_POLICY, v);
    }
}

pub fn shell(options: LeptosOptions) -> impl IntoView {
    security_policy();
    let loc = locale();
    view! {
        <!DOCTYPE html>
        <html lang=loc.tag()>
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <meta name="color-scheme" content="light dark"/>
                <title>"Profanity Watch"</title>
                <link rel="stylesheet" href="/pkg/pb.css"/>
                <HydrationScripts options islands=true/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <FlatRoutes fallback=|| view! { <Page><pages::NotFound/></Page> }>
                <Route path=StaticSegment("") view=|| view! { <Page><pages::wall::WallPage/></Page> }/>
                <Route path=StaticSegment("voice-lines") view=|| view! { <Page><pages::voicelines::VoiceLinesPage/></Page> }/>
                <Route path=StaticSegment("reports") view=|| view! { <Page><pages::reports::ReportsPage/></Page> }/>
                <Route path=StaticSegment("audit") view=|| view! { <Page><pages::audit::AuditPage/></Page> }/>
                <Route path=StaticSegment("system") view=|| view! { <Page><pages::system::SystemPage/></Page> }/>
                <Route path=StaticSegment("settings") view=|| view! { <Page><pages::settings::GlobalSettingsPage/></Page> }/>
                <Route path=(StaticSegment("settings"), ParamSegment("section")) view=|| view! { <Page><pages::settings::GlobalSettingsPage/></Page> }/>
                <Route path=(StaticSegment("c"), ParamSegment("g"), StaticSegment("settings"), ParamSegment("section")) view=|| view! { <Page><pages::community::CommunityPage/></Page> }/>
                <Route path=(StaticSegment("c"), ParamSegment("g")) view=|| view! { <Page><pages::community::CommunityPage/></Page> }/>
                <Route path=(StaticSegment("c"), ParamSegment("g"), ParamSegment("tab")) view=|| view! { <Page><pages::community::CommunityPage/></Page> }/>
                <Route path=(StaticSegment("c"), ParamSegment("g"), StaticSegment("p"), ParamSegment("u")) view=|| view! { <Page><pages::person::PersonPage/></Page> }/>
                <Route path=(StaticSegment("c"), ParamSegment("g"), StaticSegment("p"), ParamSegment("u"), ParamSegment("tab")) view=|| view! { <Page><pages::person::PersonPage/></Page> }/>
                <Route path=StaticSegment("invite") view=|| view! { <Page><pages::invite::InvitePage/></Page> }/>
                <Route path=StaticSegment("confirm") view=|| view! { <Page><pages::confirm::ConfirmPage/></Page> }/>
                <Route path=StaticSegment("setup") view=pages::setup::SetupPage/>
            </FlatRoutes>
        </Router>
    }
}

/// The layout every logged-in page shares: the sidebar and the main area.
#[component]
fn Page(children: Children) -> impl IntoView {
    let Some(v) = viewer() else {
        return view! { <pages::LoginNeeded/> }.into_any();
    };
    let loc = v.locale;
    let path = use_context::<http::request::Parts>()
        .map(|p| p.uri.path().to_owned())
        .unwrap_or_default();
    let mut sidebar = match app().engine.hub().state(&Topic::Sidebar) {
        Some(TopicState::Sidebar(s)) => s,
        _ => Default::default(),
    };
    if !v.owner {
        sidebar.communities.retain(|c| v.guilds.contains(&c.id));
    }
    let paused_everywhere = app().engine.settings().current().paused_everywhere();
    current_notice();
    let nav = |href: &'static str, id: &str| {
        let active = if href == "/" {
            path == "/"
        } else {
            path.starts_with(href)
        };
        view! { <a href=href class:active=active>{text(loc, id, &[])}</a> }
    };
    view! {
        <div class="layout">
            <aside class="sidebar">
                <a class="brand" href="/">"Profanity Watch"</a>
                <nav class="main-nav">
                    {nav("/", "ui-nav-live")}
                    {nav("/voice-lines", "ui-nav-voice-lines")}
                    {nav("/reports", "ui-nav-reports")}
                    {nav("/audit", "ui-nav-audit")}
                    {v.owner.then(|| nav("/settings", "ui-nav-settings"))}
                    {v.owner.then(|| nav("/system", "ui-nav-system"))}
                </nav>
                <h3 class="section">{text(loc, "ui-nav-communities", &[])}</h3>
                <SidebarLive initial=sidebar locale=loc current=path.clone()/>
                <a class="invite" href="/invite" class:active=path == "/invite">{text(loc, "ui-invite", &[])}</a>
                <div class="me">
                    <span class="name">{v.name.clone()}</span>
                    <form method="post" action="/auth/logout" class="row">
                        <input type="hidden" name="csrf" value=v.csrf.clone()/>
                        <input type="hidden" name="back" value=path.clone()/>
                        <button class="link">{text(loc, "ui-nav-logout", &[])}</button>
                        <button class="link muted" name="everywhere" value="1" title=text(loc, "ui-logout-everywhere-help", &[])>
                            {text(loc, "ui-logout-everywhere", &[])}
                        </button>
                    </form>
                </div>
                <SourceLink/>
            </aside>
            <main class="main">
                <NoticeBar/>
                {paused_everywhere.then(|| view! {
                    <div class="notice warn" role="status">
                        {text(loc, "ui-paused-everywhere-banner", &[])}
                        {v.owner.then(|| view! { " " <a href="/system#pause">{text(loc, "ui-paused-everywhere-where", &[])}</a> })}
                    </div>
                })}
                {children()}
            </main>
            <div class="banner offline-banner" role="status">{text(loc, "ui-offline", &[])}</div>
            <div class="overlay auth-overlay" role="alertdialog">
                <p>{text(loc, "ui-auth-expired", &[])}</p>
                <a class="button primary" href=login_href()>{text(loc, "ui-log-in-again", &[])}</a>
            </div>
        </div>
    }
    .into_any()
}

/// Where the bot's source is (AGPL-3.0: everyone who uses the page can get it).
#[component]
pub fn SourceLink() -> impl IntoView {
    view! {
        <a class="source muted small" href=env!("CARGO_PKG_REPOSITORY") rel="noopener" target="_blank">
            {t("ui-source")}
        </a>
    }
}

/// What the bot asks for when it is invited: what it needs, and the members' actions when they are on.
pub fn invite_permissions(actions: bool) -> u64 {
    use pb_fluxer_api::perms as p;
    p::BOT
        | if actions {
            p::MUTE_MEMBERS | p::MOVE_MEMBERS | p::MODERATE_MEMBERS
        } else {
            0
        }
}

/// Whether moderation actions are on in a community: for it, or for anyone in it.
pub fn actions_in(g: GuildId) -> bool {
    let tree = app().engine.settings().current();
    tree.effective(Some(g), None).actions_enabled.value
        || tree.servers.get(&g).is_some_and(|s| {
            s.people
                .keys()
                .any(|u| tree.effective(Some(g), Some(*u)).actions_enabled.value)
        })
}

/// Whether moderation actions are on anywhere (globally, or in any community or for anyone in one).
pub fn actions_anywhere() -> bool {
    let tree = app().engine.settings().current();
    tree.effective(None, None).actions_enabled.value || tree.known_guilds().into_iter().any(actions_in)
}

pub fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The page being rendered, with its query (`/c/1?before=5`).
pub fn here_with_query() -> String {
    use_context::<http::request::Parts>()
        .and_then(|p| p.uri.path_and_query().map(|pq| pq.as_str().to_owned()))
        .unwrap_or_else(|| "/".into())
}

/// "Log in", coming back to this page afterwards.
pub fn login_href() -> String {
    match here_with_query().as_str() {
        "/" => "/login".into(),
        here => format!("/login?next={}", url_encode(here)),
    }
}

/// The message the last form left, if any.
#[component]
pub fn NoticeBar() -> impl IntoView {
    let notice = current_notice();
    let loc = viewer().map_or(Locale::En, |v| v.locale);
    notice.map(|n| {
        let (class, role) = if n.ok {
            ("notice ok", "status")
        } else {
            ("notice error", "alert")
        };
        // A plain link (not a redirect from the form): the page's form-action policy covers only this site.
        let again = n.log_in_again.map(|next| {
            let href = format!("/login?next={}", url_encode(&next));
            view! { " " <a class="button" href=href>{text(loc, "ui-log-in-again", &[])}</a> }
        });
        view! { <div class=class role=role>{n.text}{again}</div> }
    })
}
