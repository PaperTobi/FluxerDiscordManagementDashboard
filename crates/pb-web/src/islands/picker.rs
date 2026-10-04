//! A box to add someone by name: a user ID or mention works as it is (a plain form, also without scripts, where the
//! server looks the name up); with scripts, typing a name lists matching members (from Fluxer: of one community, or of
//! every community this login sees), and picking one sends the form with their ID.

use leptos::html::{Form, Input};
use leptos::prelude::*;
use pb_domain::GuildId;
use pb_live_proto::Who;

use super::widgets::Avatar;

/// `action` with the `hidden` fields and the person in `field`; `guild`: search that community (else all).
#[island]
pub fn MemberPicker(
    guild: Option<GuildId>,
    action: String,
    field: String,
    hidden: Vec<(String, String)>,
    button: String,
    placeholder: String,
    #[prop(optional)] id: Option<String>,
) -> impl IntoView {
    let found: RwSignal<Vec<Who>> = RwSignal::new(Vec::new());
    let input: NodeRef<Input> = NodeRef::new();
    let form: NodeRef<Form> = NodeRef::new();
    // `data-ready` turns true once suggestions work in the browser (effects never run on the server).
    let ready = RwSignal::new(false);
    Effect::new(move |_| ready.set(true));
    #[cfg(feature = "hydrate")]
    let pending: StoredValue<Option<leptos::prelude::TimeoutHandle>, LocalStorage> = StoredValue::new_local(None);
    // Only the answer to the latest search is shown.
    #[cfg(feature = "hydrate")]
    let latest: StoredValue<u32, LocalStorage> = StoredValue::new_local(0);
    let on_input = move |_ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            let q = input.get().map(|i| i.value()).unwrap_or_default();
            if let Some(h) = pending.try_update_value(Option::take).flatten() {
                h.clear();
            }
            // Only names: an ID or mention is used as it is.
            if q.trim().is_empty() || q.trim().starts_with('<') || q.trim().chars().all(|c| c.is_ascii_digit()) {
                found.set(Vec::new());
                return;
            }
            let h = set_timeout_with_handle(
                move || {
                    let n = latest.get_value().wrapping_add(1);
                    latest.set_value(n);
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Some(list) = search(guild, &q).await
                            && latest.get_value() == n
                        {
                            found.set(list);
                        }
                    });
                },
                std::time::Duration::from_millis(250),
            )
            .ok();
            pending.set_value(h);
        }
    };
    // On the server only the browser's search uses the community.
    #[cfg(not(feature = "hydrate"))]
    let _ = guild;
    let pick = move |user: String| {
        if let Some(i) = input.get() {
            i.set_value(&user);
        }
        if let Some(f) = form.get() {
            let _ = f.request_submit();
        }
    };
    view! {
        <form method="post" action=action class="row picker" node_ref=form data-ready=move || ready.get().to_string()>
            {hidden.into_iter().map(|(k, v)| view! { <input type="hidden" name=k value=v/> }).collect_view()}
            <input id=id name=field required autocomplete="off" node_ref=input on:input=on_input placeholder=placeholder/>
            <button class="button primary">{button}</button>
        </form>
        <Show when=move || !found.with(Vec::is_empty)>
            <ul class="suggest">
                <For each=move || found.get() key=|w| w.user let:w>
                    {
                        let id = w.user.to_string();
                        view! {
                            <li>
                                <button type="button" class="link" on:click=move |_| pick(id.clone())>
                                    <Avatar who=w.clone() size=22/>
                                    {w.name.clone()}
                                </button>
                            </li>
                        }
                    }
                </For>
            </ul>
        </Show>
    }
}

#[cfg(feature = "hydrate")]
async fn search(guild: Option<GuildId>, q: &str) -> Option<Vec<Who>> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window()?;
    let q: String = js_sys::encode_uri_component(q).into();
    let url = match guild {
        Some(g) => format!("/api/members?guild={g}&q={q}"),
        None => format!("/api/members?q={q}"),
    };
    let res: web_sys::Response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !res.ok() {
        return None;
    }
    let body = JsFuture::from(res.text().ok()?).await.ok()?.as_string()?;
    serde_json::from_str(&body).ok()
}
