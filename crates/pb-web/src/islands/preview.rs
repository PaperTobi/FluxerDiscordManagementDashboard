//! A voice line's preview: as the bot would say it there now, or (with scripts) in a language picked here.

use leptos::prelude::*;
use pb_i18n::{Locale, text};

/// `src`: the preview address without a language; `langs`: (code, name) of the languages offered.
#[island]
pub fn PreviewPlayer(src: String, langs: Vec<(String, String)>, locale: Locale) -> impl IntoView {
    let t = move |id: &str| text(locale, id, &[]);
    let lang = RwSignal::new(String::new());
    // The language choice needs scripts: hidden until they run.
    let ready = RwSignal::new(false);
    Effect::new(move |_| ready.set(true));
    let base = StoredValue::new(src);
    let url = move || match lang.get() {
        l if l.is_empty() => base.get_value(),
        l => format!("{}&lang={l}", base.get_value()),
    };
    view! {
        <span class="preview row">
            <select hidden=move || !ready.get() title=t("ui-preview-language") on:change=move |ev| lang.set(event_target_value(&ev))>
                <option value="">{t("ui-preview-as-bot")}</option>
                {langs.into_iter().map(|(code, name)| view! { <option value=code>{name}</option> }).collect_view()}
            </select>
            <audio controls preload="none" src=url title=t("ui-preview")></audio>
        </span>
    }
}
