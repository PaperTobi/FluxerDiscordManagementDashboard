//! The web UI. Pages are rendered on the server (Leptos SSR); the live parts and the editors are islands, hydrated in
//! the browser, sharing one live connection. Changes go through plain HTML forms (they work without scripts) and a
//! few JSON endpoints of `pb-web-server`.

pub mod fmt;
pub mod islands;
pub mod live;

#[cfg(feature = "ssr")]
pub mod app;
#[cfg(feature = "ssr")]
pub mod pages;

/// Entry point of the browser bundle.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    // A panic shows in the browser console with the JavaScript stack.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&js_sys::Error::new(&info.to_string()));
    }));
    leptos::mount::hydrate_islands();
}
