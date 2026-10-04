//! `/invite`: what inviting the bot does, the invite link (with the permissions the bot needs, and the members'
//! actions when they are on anywhere), and a link per community that asks again for what the bot lacks there.

use leptos::prelude::*;
use pb_domain::GuildId;
use pb_engine::Connection;
use pb_i18n::text;
use pb_live_proto::{Topic, TopicState};

use super::community::permission_rows;
use crate::app::{actions_anywhere, actions_in, app, invite_permissions, viewer};
use crate::islands::CopyText;

/// The authorize link for one community the bot is in (Fluxer preselects it): asks again for what the bot needs
/// there. `None` while the bot does not know its instance or token.
pub fn reauthorize_url(guild: GuildId) -> Option<String> {
    app()
        .host
        .invite_url(invite_permissions(actions_in(guild)))
        .map(|url| format!("{url}&guild_id={guild}"))
}

#[component]
pub fn InvitePage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let t = move |id: &str| text(loc, id, &[]);
    let engine = app().engine;
    let connected = engine.connection() == Connection::Ready;
    let url = app().host.invite_url(invite_permissions(actions_anywhere()));
    // The communities this login sees where the bot lacks something.
    let mut communities = match engine.hub().state(&Topic::Sidebar) {
        Some(TopicState::Sidebar(s)) => s.communities,
        _ => Vec::new(),
    };
    communities.retain(|c| v.may_see(c.id) && c.available);
    let lacking: Vec<(GuildId, String, Vec<String>)> = communities
        .into_iter()
        .filter_map(|c| {
            let missing: Vec<String> = permission_rows(loc, c.id)
                .into_iter()
                .filter(|(_, m)| !m.is_empty())
                .map(|(what, m)| format!("{what}: {}", m.join(", ")))
                .collect();
            (!missing.is_empty()).then_some((c.id, c.name, missing))
        })
        .collect();
    view! {
        <header class="page-head"><h1>{t("ui-invite-title")}</h1></header>
        {(!connected).then(|| view! { <p class="notice warn" role="status">{t("ui-invite-not-connected")}</p> })}
        <section class="card">
            <h2>{t("ui-invite-what")}</h2>
            <p>{t("ui-invite-help")}</p>
            <p class="muted small">{t("ui-invite-permissions")}</p>
            {match url {
                Some(url) => view! {
                    <CopyText text=url.clone() locale=loc/>
                    <p><a class="button primary" href=url target="_blank" rel="noopener">{t("ui-invite-open")}</a></p>
                }
                .into_any(),
                None => view! { <p class="muted">{t("ui-invite-no-link")}</p> }.into_any(),
            }}
        </section>
        {(!lacking.is_empty()).then(|| view! {
            <section class="card">
                <h2>{t("ui-reauthorize-title")}</h2>
                <p class="muted">{t("ui-reauthorize-help")}</p>
                <table class="rows">
                    <tbody>
                        {lacking.into_iter().map(|(g, name, missing)| view! {
                            <tr>
                                <td><a href=format!("/c/{g}")>{name}</a></td>
                                <td>{missing.into_iter().map(|m| view! { <div class="small">{m}</div> }).collect_view()}</td>
                                <td class="right">
                                    {reauthorize_url(g).map(|url| view! {
                                        <a class="button" href=url target="_blank" rel="noopener">{t("ui-reauthorize")}</a>
                                    })}
                                </td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </section>
        })}
    }
    .into_any()
}
