//! Recording a clip in the browser (MediaRecorder): name it, record, stop, and it is uploaded like a file (without a
//! name it is called after the time it was made). The browser only allows the microphone on https (or localhost);
//! elsewhere this says so and the upload form remains.

use leptos::html::Input;
use leptos::prelude::*;
use pb_i18n::{Locale, text};

#[derive(Debug, Clone, PartialEq, Eq)]
// On the server only the first state is ever drawn.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
enum State {
    Idle,
    Recording,
    Uploading,
    Failed(String),
}

#[island]
pub fn ClipRecorder(csrf: String, locale: Locale) -> impl IntoView {
    let state = RwSignal::new(State::Idle);
    let csrf = StoredValue::new(csrf);
    let name: NodeRef<Input> = NodeRef::new();
    let t = move |id: &str| text(locale, id, &[]);
    // `data-ready` turns true once the recorder works in the browser (effects never run on the server). Browsers
    // only give the microphone to secure pages (https, or localhost).
    let ready = RwSignal::new(false);
    let insecure = RwSignal::new(false);
    Effect::new(move |_| {
        #[cfg(feature = "hydrate")]
        insecure.set(!web_sys::window().is_some_and(|w| w.is_secure_context()));
        ready.set(true);
    });
    #[cfg(feature = "hydrate")]
    let session: StoredValue<Option<rec::Session>, LocalStorage> = StoredValue::new_local(None);
    let toggle = move |_| {
        #[cfg(feature = "hydrate")]
        {
            match state.get_untracked() {
                State::Recording => {
                    if let Some(s) = session.try_update_value(Option::take).flatten() {
                        state.set(State::Uploading);
                        let typed = name.get().map(|i| i.value()).unwrap_or_default();
                        let named = if typed.trim().is_empty() {
                            text(locale, "ui-record-default-name", &[("when", rec::now_local().into())])
                        } else {
                            typed.trim().to_owned()
                        };
                        s.stop(named);
                    }
                }
                State::Uploading => {}
                _ => {
                    state.set(State::Recording);
                    let token = csrf.get_value();
                    let words = rec::Words {
                        refused: t("ui-record-refused"),
                        upload_failed: t("ui-record-upload-failed"),
                    };
                    wasm_bindgen_futures::spawn_local(async move {
                        let done = move |r: Result<(), String>| match r {
                            Ok(()) => {
                                if let Some(w) = web_sys::window() {
                                    let _ = w.location().reload();
                                }
                            }
                            Err(e) => state.set(State::Failed(e)),
                        };
                        match rec::Session::start(token, words, done).await {
                            Ok(s) => session.set_value(Some(s)),
                            Err(e) => state.set(State::Failed(e)),
                        }
                    });
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (csrf, state, name);
    };
    view! {
        <div class="recorder row" data-ready=move || ready.get().to_string()>
            <input node_ref=name placeholder=t("ui-clip-name") aria-label=t("ui-clip-name")
                disabled=move || insecure.get() || state.get() == State::Uploading/>
            <button class="button" class:recording=move || state.get() == State::Recording on:click=toggle
                disabled=move || insecure.get() || state.get() == State::Uploading>
                {move || match state.get() {
                    State::Recording => t("ui-record-stop"),
                    State::Uploading => t("ui-record-uploading"),
                    _ => t("ui-record"),
                }}
            </button>
            {move || match state.get() {
                _ if insecure.get() => view! { <span class="muted small">{t("ui-record-needs-https")}</span> }.into_any(),
                State::Failed(e) => view! { <span class="notice error">{e}</span> }.into_any(),
                State::Recording => view! { <span class="muted">{t("ui-record-hint")}</span> }.into_any(),
                _ => ().into_any(),
            }}
        </div>
    }
}

#[cfg(feature = "hydrate")]
mod rec {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Blob, BlobEvent, FormData, MediaRecorder, MediaStream, MediaStreamConstraints, RequestInit};

    /// The browser's own words for an error (it words them in its language).
    fn js(e: &wasm_bindgen::JsValue) -> Option<String> {
        e.as_string().or_else(|| {
            js_sys::Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
    }

    /// What to say when things fail (in the page's language).
    #[derive(Clone)]
    pub struct Words {
        /// The browser gave no reason.
        pub refused: String,
        pub upload_failed: String,
    }

    /// A recording in progress.
    pub struct Session {
        recorder: MediaRecorder,
        /// The clip's name, given when it stops.
        name: Rc<RefCell<String>>,
    }

    /// The local date and time (`2026-10-04 21:40`), for naming a recording.
    pub fn now_local() -> String {
        let d = js_sys::Date::new_0();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            d.get_full_year(),
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }

    impl Session {
        /// Starts recording.
        pub async fn start(
            csrf: String,
            words: Words,
            done: impl Fn(Result<(), String>) + 'static,
        ) -> Result<Session, String> {
            let js = |e: wasm_bindgen::JsValue| js(&e).unwrap_or_else(|| words.refused.clone());
            let window = web_sys::window().ok_or_else(|| words.refused.clone())?;
            let devices = window.navigator().media_devices().map_err(js)?;
            let constraints = MediaStreamConstraints::new();
            constraints.set_audio(&true.into());
            let stream: MediaStream =
                JsFuture::from(devices.get_user_media_with_constraints(&constraints).map_err(js)?)
                    .await
                    .map_err(js)?
                    .dyn_into()
                    .map_err(js)?;
            let recorder = MediaRecorder::new_with_media_stream(&stream).map_err(js)?;
            let chunks: Rc<RefCell<Vec<Blob>>> = Rc::default();
            let on_data = {
                let chunks = chunks.clone();
                Closure::<dyn FnMut(BlobEvent)>::new(move |e: BlobEvent| {
                    if let Some(b) = e.data() {
                        chunks.borrow_mut().push(b);
                    }
                })
            };
            recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
            // The data handler lives until the recording stopped (the stop handler then drops it); the stop handler
            // runs once and is freed by the browser after that.
            let on_data = Rc::new(RefCell::new(Some(on_data)));
            let rec = recorder.clone();
            let upload_words = words.clone();
            let name: Rc<RefCell<String>> = Rc::default();
            let named = name.clone();
            let on_stop = Closure::once_into_js(move || {
                rec.set_ondataavailable(None);
                drop(on_data.borrow_mut().take());
                // Free the microphone.
                for t in stream.get_tracks().iter() {
                    if let Ok(t) = t.dyn_into::<web_sys::MediaStreamTrack>() {
                        t.stop();
                    }
                }
                let parts = js_sys::Array::new();
                for b in chunks.borrow().iter() {
                    parts.push(b);
                }
                let kind = rec.mime_type();
                let name = named.borrow().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    done(upload(parts, kind, csrf, name, &upload_words).await);
                });
            });
            recorder.set_onstop(Some(on_stop.unchecked_ref()));
            recorder.start().map_err(js)?;
            Ok(Session { recorder, name })
        }

        /// Stops recording; the clip is uploaded as `name`.
        pub fn stop(self, name: String) {
            *self.name.borrow_mut() = name;
            let _ = self.recorder.stop();
        }
    }

    async fn upload(
        parts: js_sys::Array,
        kind: String,
        csrf: String,
        name: String,
        words: &Words,
    ) -> Result<(), String> {
        let js = |e: wasm_bindgen::JsValue| js(&e).unwrap_or_else(|| words.refused.clone());
        let bag = web_sys::BlobPropertyBag::new();
        bag.set_type(&kind);
        let blob = Blob::new_with_blob_sequence_and_options(&parts, &bag).map_err(js)?;
        let ext = if kind.contains("ogg") {
            "ogg"
        } else if kind.contains("mp4") {
            "m4a"
        } else {
            "webm"
        };
        let form = FormData::new().map_err(js)?;
        form.append_with_str("csrf", &csrf).map_err(js)?;
        form.append_with_str("back", "/voice-lines").map_err(js)?;
        form.append_with_str("name", &name).map_err(js)?;
        form.append_with_str("lang", "").map_err(js)?;
        form.append_with_blob_and_filename("file", &blob, &format!("recording.{ext}"))
            .map_err(js)?;
        let init = RequestInit::new();
        init.set_method("POST");
        init.set_body(&form);
        let window = web_sys::window().ok_or_else(|| words.refused.clone())?;
        let res: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init("/clips", &init))
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        if res.ok() {
            Ok(())
        } else {
            Err(format!("{} ({})", words.upload_failed, res.status()))
        }
    }
}
