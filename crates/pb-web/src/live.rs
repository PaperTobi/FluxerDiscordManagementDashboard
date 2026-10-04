//! The page's one connection to the bot's live updates (`/live`), shared by every island on the page.
//!
//! Hidden tabs keep only the sidebar; a tab that comes back starts a new view and gets fresh snapshots; a frozen or
//! hidden-and-discarded page closes its socket and reconnects when it is shown again; an ended login shows "log in
//! again" instead of reconnecting forever. A socket that stays silent (the server pings every 15 s; a dead network
//! path never says so) is replaced after 45 s, and a computer that comes back online reconnects at once.

use pb_live_proto::{Topic, TopicState};

/// A watch on a topic; dropping it stops watching.
pub struct Watch {
    #[cfg(feature = "hydrate")]
    id: u64,
    #[cfg(feature = "hydrate")]
    topic: Topic,
}

impl Watch {
    /// Stops watching (the same as dropping it).
    pub fn stop(self) {}
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watch")
    }
}

#[cfg(not(feature = "hydrate"))]
mod imp {
    use super::{Topic, TopicState, Watch};

    /// On the server nothing is live.
    pub fn watch(_topic: Topic, _on: impl Fn(&TopicState) + 'static) -> Watch {
        Watch {}
    }

    /// The bot's clock (on the server: the system clock).
    pub fn server_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
    }
}

#[cfg(feature = "hydrate")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use pb_live_proto::{ClientMsg, ClockOffset, Outcome, ServerMsg, TopicTracker};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

    use super::{Topic, TopicState, Watch};

    type Listener = Rc<dyn Fn(&TopicState)>;

    /// A socket without a frame for this long is taken for dead (three of the server's pings missed).
    const SILENT_MS: i64 = 45_000;
    /// How often the silence is checked.
    const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

    struct Client {
        tracker: RefCell<TopicTracker>,
        ws: RefCell<Option<WebSocket>>,
        listeners: RefCell<HashMap<Topic, Vec<(u64, Listener)>>>,
        next_id: Cell<u64>,
        clock: RefCell<ClockOffset>,
        failures: Cell<u32>,
        /// When the socket last opened or brought a frame (ms, local clock).
        last_frame: Cell<i64>,
        /// The login ended: no reconnecting.
        ended: Cell<bool>,
        /// The page is frozen or being left: no reconnecting until it is shown again.
        frozen: Cell<bool>,
        // The socket's handlers live as long as the socket.
        handlers: RefCell<Vec<Box<dyn std::any::Any>>>,
    }

    thread_local! {
        static CLIENT: RefCell<Option<Rc<Client>>> = const { RefCell::new(None) };
    }

    fn now_ms() -> i64 {
        js_sys::Date::now() as i64
    }

    fn document() -> Option<web_sys::Document> {
        web_sys::window().and_then(|w| w.document())
    }

    fn visible() -> bool {
        document().is_none_or(|d| d.visibility_state() == web_sys::VisibilityState::Visible)
    }

    fn client() -> Rc<Client> {
        CLIENT.with(|c| {
            if let Some(c) = c.borrow().as_ref() {
                return c.clone();
            }
            let new = Rc::new(Client {
                tracker: RefCell::new(TopicTracker::new(visible())),
                ws: RefCell::new(None),
                listeners: RefCell::new(HashMap::new()),
                next_id: Cell::new(1),
                clock: RefCell::new(ClockOffset::default()),
                failures: Cell::new(0),
                last_frame: Cell::new(now_ms()),
                ended: Cell::new(false),
                frozen: Cell::new(false),
                handlers: RefCell::new(Vec::new()),
            });
            *c.borrow_mut() = Some(new.clone());
            page_events(&new);
            watchdog(&new);
            connect(&new);
            new
        })
    }

    fn send(c: &Client, m: &ClientMsg) {
        if let (Some(ws), Ok(text)) = (c.ws.borrow().as_ref(), serde_json::to_string(m))
            && ws.ready_state() == WebSocket::OPEN
        {
            let _ = ws.send_with_str(&text);
        }
    }

    fn set_body_class(class: &str, on: bool) {
        if let Some(b) = document().and_then(|d| d.body()) {
            let list = b.class_list();
            let _ = if on { list.add_1(class) } else { list.remove_1(class) };
        }
    }

    /// Opens the socket, unless one is open or opening (a reconnect timer and the `online` event may both ask).
    fn connect(c: &Rc<Client>) {
        if c.ended.get() || c.frozen.get() || c.ws.borrow().is_some() {
            return;
        }
        c.last_frame.set(now_ms());
        let Some(loc) = web_sys::window().map(|w| w.location()) else {
            return;
        };
        let scheme = if loc.protocol().ok().as_deref() == Some("https:") {
            "wss"
        } else {
            "ws"
        };
        let Ok(host) = loc.host() else { return };
        let Ok(ws) = WebSocket::new(&format!("{scheme}://{host}/live")) else {
            reconnect_later(c);
            return;
        };
        let (c1, c2, c3) = (c.clone(), c.clone(), c.clone());
        let onopen = Closure::<dyn FnMut(Event)>::new(move |_| {
            c1.failures.set(0);
            c1.last_frame.set(now_ms());
            set_body_class("offline", false);
            let hello = c1.tracker.borrow_mut().hello();
            send(&c1, &hello);
        });
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            c2.last_frame.set(now_ms());
            let Some(text) = e.data().as_string() else { return };
            match serde_json::from_str::<ServerMsg>(&text) {
                Ok(m) => receive(&c2, m),
                Err(err) => leptos::logging::warn!("an unreadable live frame: {err}"),
            }
        });
        let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |_| {
            *c3.ws.borrow_mut() = None;
            if !c3.ended.get() && !c3.frozen.get() {
                set_body_class("offline", true);
                reconnect_later(&c3);
            }
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        {
            let mut h = c.handlers.borrow_mut();
            h.clear();
            h.push(Box::new(onopen));
            h.push(Box::new(onmessage));
            h.push(Box::new(onclose));
        }
        *c.ws.borrow_mut() = Some(ws);
    }

    fn reconnect_later(c: &Rc<Client>) {
        let n = c.failures.get();
        c.failures.set(n + 1);
        let delay = (1000u32 << n.min(5)).min(30_000);
        let c2 = c.clone();
        leptos::prelude::set_timeout(move || connect(&c2), std::time::Duration::from_millis(u64::from(delay)));
    }

    /// Drops the socket (whatever state it is in) and opens a new one now.
    fn replace_socket(c: &Rc<Client>) {
        if c.ended.get() || c.frozen.get() {
            return;
        }
        close(c);
        set_body_class("offline", true);
        connect(c);
    }

    /// Replaces a socket that has been silent too long: a connection the network dropped without a word (a laptop
    /// that slept, a router that forgot it) looks open forever otherwise.
    fn watchdog(c: &Rc<Client>) {
        let c = c.clone();
        let check = move || {
            let open = c.ws.borrow().is_some();
            if open && now_ms() - c.last_frame.get() > SILENT_MS {
                leptos::logging::warn!("the live connection was silent too long; connecting again");
                replace_socket(&c);
            }
        };
        // The interval lives as long as the page.
        let _ = leptos::prelude::set_interval_with_handle(check, WATCH_EVERY);
    }

    fn close(c: &Client) {
        if let Some(ws) = c.ws.borrow_mut().take() {
            ws.set_onclose(None);
            let _ = ws.close();
        }
    }

    fn receive(c: &Rc<Client>, m: ServerMsg) {
        let outcome = c.tracker.borrow_mut().receive(m);
        match outcome {
            Outcome::Changed(topic) => {
                let state = c.tracker.borrow().state(&topic).cloned();
                if let Some(state) = state {
                    let ls: Vec<Listener> = c
                        .listeners
                        .borrow()
                        .get(&topic)
                        .map(|l| l.iter().map(|(_, f)| f.clone()).collect())
                        .unwrap_or_default();
                    for f in ls {
                        f(&state);
                    }
                }
            }
            Outcome::Send(msg) | Outcome::Broken(_, msg) => send(c, &msg),
            Outcome::Retry(msgs) => {
                for msg in msgs {
                    send(c, &msg);
                }
            }
            Outcome::Welcome { server_ms, .. } => c.clock.borrow_mut().observe(server_ms, now_ms(), None),
            Outcome::Ping {
                pong,
                server_ms,
                rtt_ms,
            } => {
                c.clock.borrow_mut().observe(server_ms, now_ms(), rtt_ms);
                send(c, &pong);
            }
            Outcome::AuthExpired => {
                c.ended.set(true);
                close(c);
                set_body_class("auth-expired", true);
            }
            Outcome::Shutdown { .. } => {
                close(c);
                set_body_class("offline", true);
                reconnect_later(c);
            }
            Outcome::Reload(_) => {
                if let Some(w) = web_sys::window() {
                    let _ = w.location().reload();
                }
            }
            Outcome::Denied(..) | Outcome::Ignored => {}
        }
    }

    /// Hidden tabs, frozen pages, pages being left, and a computer that is online again.
    fn page_events(c: &Rc<Client>) {
        let Some(doc) = document() else { return };
        let Some(win) = web_sys::window() else { return };
        // Back online (another network, out of sleep): the old socket is likely dead, and waiting out the backoff
        // would only delay; connect now.
        let c0 = c.clone();
        let on_online = Closure::<dyn FnMut(Event)>::new(move |_| {
            c0.failures.set(0);
            replace_socket(&c0);
        });
        let _ = win.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
        on_online.forget();
        let c1 = c.clone();
        let on_visibility = Closure::<dyn FnMut(Event)>::new(move |_| {
            let msg = c1.tracker.borrow_mut().set_visible(visible());
            if let Some(msg) = msg {
                send(&c1, &msg);
            }
        });
        let _ = doc.add_event_listener_with_callback("visibilitychange", on_visibility.as_ref().unchecked_ref());
        on_visibility.forget();
        for (target, ev) in [("doc", "freeze"), ("win", "pagehide")] {
            let c2 = c.clone();
            let f = Closure::<dyn FnMut(Event)>::new(move |_| {
                c2.frozen.set(true);
                close(&c2);
            });
            let _ = if target == "doc" {
                doc.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref())
            } else {
                win.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref())
            };
            f.forget();
        }
        for (target, ev) in [("doc", "resume"), ("win", "pageshow")] {
            let c2 = c.clone();
            let f = Closure::<dyn FnMut(Event)>::new(move |_| {
                if c2.frozen.replace(false) && c2.ws.borrow().is_none() {
                    connect(&c2);
                }
            });
            let _ = if target == "doc" {
                doc.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref())
            } else {
                win.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref())
            };
            f.forget();
        }
    }

    /// Calls `on` with the topic's state now (when known) and on every change.
    pub fn watch(topic: Topic, on: impl Fn(&TopicState) + 'static) -> Watch {
        let c = client();
        let id = c.next_id.get();
        c.next_id.set(id + 1);
        let on: Listener = Rc::new(on);
        c.listeners
            .borrow_mut()
            .entry(topic.clone())
            .or_default()
            .push((id, on.clone()));
        if let Some(state) = c.tracker.borrow().state(&topic).cloned() {
            on(&state);
        }
        let sub = c.tracker.borrow_mut().want(topic.clone());
        if let Some(m) = sub {
            send(&c, &m);
        }
        Watch { id, topic }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            CLIENT.with(|c| {
                let Some(c) = c.borrow().clone() else { return };
                let empty = {
                    let mut ls = c.listeners.borrow_mut();
                    let list = ls.entry(self.topic.clone()).or_default();
                    list.retain(|(i, _)| *i != self.id);
                    list.is_empty()
                };
                if empty {
                    c.listeners.borrow_mut().remove(&self.topic);
                    let m = c.tracker.borrow_mut().unwant(&self.topic);
                    if let Some(m) = m {
                        send(&c, &m);
                    }
                }
            });
        }
    }

    /// The bot's clock in milliseconds (the local clock corrected by the measured offset).
    pub fn server_now() -> i64 {
        CLIENT.with(|c| {
            c.borrow()
                .as_ref()
                .map_or_else(now_ms, |c| c.clock.borrow().server_now(now_ms()))
        })
    }
}

pub use imp::{server_now, watch};
