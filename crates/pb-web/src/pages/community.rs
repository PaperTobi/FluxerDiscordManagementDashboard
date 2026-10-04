//! `/c/:g` and its tabs: Overview (live calls, tracked people, violations), Voice lines, Settings, Reports.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use pb_domain::{GuildId, Scope};
use pb_i18n::{Locale, text};
use pb_live::CellSource;
use pb_live_proto::{Topic, TopicState};

use super::settings::SettingsForm;
use super::{NotFound, Tabs};
use crate::app::{app, viewer};
use crate::islands::{GuildLive, MemberPicker};

#[component]
pub fn CommunityPage() -> impl IntoView {
    let params = use_params_map();
    let g = params
        .with_untracked(|p| p.get("g"))
        .and_then(|s| s.parse::<u64>().ok())
        .map(GuildId);
    let tab = params.with_untracked(|p| p.get("tab")).unwrap_or_default();
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let Some(g) = g.filter(|g| v.may_see(*g)) else {
        return view! { <NotFound/> }.into_any();
    };
    let engine = app().engine;
    let topic = Topic::Guild { guild: g };
    if !engine.cells().ensure(&topic) {
        return view! { <NotFound/> }.into_any();
    }
    let Some(TopicState::Guild(state)) = engine.hub().state(&topic) else {
        return view! { <NotFound/> }.into_any();
    };
    let loc = v.locale;
    let base = format!("/c/{g}");
    let here = if tab.is_empty() {
        base.clone()
    } else {
        format!("{base}/{tab}")
    };
    let tabs = [
        ("", "ui-tab-overview"),
        ("voice-lines", "ui-nav-voice-lines"),
        ("settings", "ui-tab-settings"),
        ("reports", "ui-nav-reports"),
    ]
    .into_iter()
    .map(|(t, id)| {
        let href = if t.is_empty() {
            base.clone()
        } else {
            format!("{base}/{t}")
        };
        (href, text(loc, id, &[]), t == tab)
    })
    .collect::<Vec<_>>();
    let body = match tab.as_str() {
        "" => view! {
            <GuildLive initial=(*state).clone() locale=loc csrf=v.csrf.clone() back=here.clone()/>
            <section class="card">
                <h2>{text(loc, "ui-track-someone", &[])}</h2>
                <MemberPicker
                    guild=Some(g)
                    action="/people/track".to_owned()
                    field="user".to_owned()
                    hidden=vec![
                        ("csrf".to_owned(), v.csrf.clone()),
                        ("back".to_owned(), here.clone()),
                        ("guild".to_owned(), g.to_string()),
                    ]
                    button=text(loc, "ui-track", &[])
                    placeholder=text(loc, "ui-user-id-or-mention", &[])
                />
                <p class="muted small">{text(loc, "ui-track-help", &[])}</p>
            </section>
            <PermissionCheck guild=g/>
            <section class="card"><super::commands::ChatCommands guild=Some(g) locale=loc/></section>
        }
        .into_any(),
        "settings" => view! { <SettingsForm scope=Scope::Server { guild: g } back=here.clone()/> }.into_any(),
        "voice-lines" => {
            view! { <super::voicelines::VoiceLinesEditor scope=Scope::Server { guild: g } back=here.clone()/> }
                .into_any()
        }
        "reports" => view! {
            <super::sentences::SentenceList guild=Some(g) user=None kind=pb_store_api::SentenceKind::Violations/>
            <super::reports::Jar guild=Some(g)/>
        }
        .into_any(),
        _ => view! { <NotFound/> }.into_any(),
    };
    // While the owner paused the whole bot, a community's own switch changes nothing: say so instead of offering it.
    let everywhere = engine.settings().current().paused_everywhere();
    let switch = (!everywhere).then(|| {
        view! {
            <form method="post" action="/settings">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value=here.clone()/>
                <input type="hidden" name="scope" value=format!("server:{g}")/>
                <input type="hidden" name="key" value="paused"/>
                <input type="hidden" name="value" value=if state.paused { "off" } else { "on" }/>
                <input type="hidden" name="action" value="set"/>
                <button class="button">{text(loc, if state.paused { "ui-resume-here" } else { "ui-pause-here" }, &[])}</button>
            </form>
        }
    });
    view! {
        <header class="page-head">
            <h1>{state.name.clone()}</h1>
            {match (everywhere, state.paused) {
                (true, _) => Some(view! { <span class="chip warn">{text(loc, "ui-paused-everywhere", &[])}</span> }),
                (false, true) => Some(view! { <span class="chip warn">{text(loc, "ui-paused", &[])}</span> }),
                (false, false) => None,
            }}
            {(!state.available).then(|| view! { <span class="chip">{text(loc, "ui-unavailable", &[])}</span> })}
            <span class="grow"></span>
            {switch}
        </header>
        <Tabs items=tabs/>
        {body}
    }
    .into_any()
}

/// What the bot needs in a community and what it lacks: each voice channel, the mod-log channel and (when actions
/// are on there) the members' actions, with the names of the missing permissions.
pub(crate) fn permission_rows(loc: Locale, guild: GuildId) -> Vec<(String, Vec<String>)> {
    use pb_fluxer_api::perms as p;
    let engine = app().engine;
    let eff = engine.settings().current().effective(Some(guild), None);
    let info = engine.guilds().get(guild).cloned();
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    let check = |channel: Option<pb_domain::ChannelId>, need: u64| {
        crate::fmt::missing_permissions(loc, engine.bot_permissions(guild, channel), need)
    };
    if let Some(info) = &info {
        let mut voice: Vec<_> = info
            .channels
            .values()
            .filter(|c| c.kind == pb_fluxer_api::ChannelKind::Voice)
            .collect();
        voice.sort_by_key(|c| c.position);
        for c in voice {
            rows.push((format!("🔊 {}", c.name), check(Some(c.id), p::VOICE)));
        }
    }
    if let Some(c) = eff.modlog_channel.value {
        let name = engine.guilds().channel_name(guild, c);
        rows.push((
            text(loc, "ui-perm-modlog", &[("channel", name.into())]),
            check(Some(c), p::VIEW_CHANNEL | p::SEND_MESSAGES | p::ATTACH_FILES),
        ));
    }
    if crate::app::actions_in(guild) {
        rows.push((
            text(loc, "ui-perm-actions", &[]),
            check(None, p::MUTE_MEMBERS | p::MOVE_MEMBERS | p::MODERATE_MEMBERS),
        ));
    }
    rows
}

/// What the bot may do in each voice channel, in the mod-log channel and (when actions are on) to members, with a link
/// that asks for what is missing.
#[component]
fn PermissionCheck(guild: GuildId) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let rows = permission_rows(loc, guild);
    let missing = rows.iter().any(|(_, m)| !m.is_empty());
    let again = missing.then(|| super::invite::reauthorize_url(guild)).flatten();
    view! {
        <section class="card">
            <h2>{text(loc, "ui-permissions", &[])}</h2>
            <table class="rows">
                <tbody>
                    {rows.into_iter().map(|(what, missing)| {
                        let ok = missing.is_empty();
                        let status = if ok {
                            text(loc, "ui-perm-ok", &[])
                        } else {
                            text(loc, "ui-perm-missing", &[("missing", missing.join(", ").into())])
                        };
                        view! {
                            <tr>
                                <td>{what}</td>
                                <td><span class=if ok { "chip ok" } else { "chip bad" }>{status}</span></td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
            <p class="muted small">{text(loc, "ui-permissions-help", &[])}</p>
            {again.map(|url| view! {
                <p><a class="button" href=url target="_blank" rel="noopener">{text(loc, "ui-reauthorize", &[])}</a></p>
            })}
        </section>
    }
    .into_any()
}
