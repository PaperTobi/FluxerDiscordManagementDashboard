//! A text to copy (a link): shown in full and selected with one click; with scripts on a secure page (https or
//! localhost) also a button that puts it on the clipboard.

use leptos::prelude::*;
use pb_i18n::Locale;

#[island]
pub fn CopyText(text: String, locale: Locale) -> impl IntoView {
    let t = move |id: &str| pb_i18n::text(locale, id, &[]);
    // Browsers only give the clipboard to secure pages: elsewhere the text stays selectable (one click selects it).
    let ready = RwSignal::new(false);
    let copied = RwSignal::new(false);
    Effect::new(move |_| {
        #[cfg(feature = "hydrate")]
        ready.set(web_sys::window().is_some_and(|w| w.is_secure_context()));
    });
    let value = StoredValue::new(text.clone());
    let copy = move |_| {
        #[cfg(feature = "hydrate")]
        if let Some(w) = web_sys::window() {
            let done = w.navigator().clipboard().write_text(&value.get_value());
            wasm_bindgen_futures::spawn_local(async move {
                copied.set(wasm_bindgen_futures::JsFuture::from(done).await.is_ok());
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (value, copied);
    };
    view! {
        <div class="copy row" data-ready=move || ready.get().to_string()>
            <code class="grow">{text}</code>
            <button type="button" class="button" hidden=move || !ready.get() on:click=copy>
                {move || if copied.get() { t("ui-copied") } else { t("ui-copy") }}
            </button>
        </div>
    }
}
