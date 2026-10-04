//! `GET /live`: the websocket every page shares (protocol in `pb-live-proto`, sessions in `pb-live`).

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use http::{HeaderMap, StatusCode};
use pb_live::SessionCtx;
use pb_live_proto::{ClientMsg, ServerMsg};

use super::access::LoginAccess;
use super::server::WebState;
use super::util::same_origin;

pub async fn live(State(st): State<WebState>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    // Browsers always send Origin with a websocket; another site's page must not ride on this login.
    if headers.get(http::header::ORIGIN).is_none() || !same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let login = st.sessions.lookup(&headers).map(|s| s.key);
    ws.on_upgrade(move |socket| run(st, login, socket))
}

async fn run(st: WebState, login: Option<String>, socket: WebSocket) {
    let (mut sink, stream) = socket.split();
    let Some(key) = login else {
        // Not logged in (or the login ended): say so; the page shows "log in again" instead of reconnecting.
        if let Ok(t) = serde_json::to_string(&ServerMsg::AuthExpired) {
            let _ = sink.send(Message::Text(t.into())).await;
        }
        let _ = sink.close().await;
        return;
    };
    let incoming = stream
        .take_while(|m| std::future::ready(matches!(m, Ok(m) if !matches!(m, Message::Close(_)))))
        .filter_map(|m| {
            std::future::ready(match m {
                Ok(Message::Text(t)) => serde_json::from_str::<ClientMsg>(&t).ok(),
                _ => None,
            })
        });
    let outgoing = sink.with(|m: ServerMsg| {
        std::future::ready(
            serde_json::to_string(&m)
                .map(|t| Message::Text(t.into()))
                .map_err(axum::Error::new),
        )
    });
    let ctx = SessionCtx {
        hub: st.engine.hub().clone(),
        access: Arc::new(LoginAccess {
            sessions: st.sessions.clone(),
            key,
        }),
        source: Arc::new(st.engine.cells()),
        cfg: st.cfg.live.clone(),
        version: st.version.clone(),
        shutdown: st.shutdown.clone(),
    };
    let end = pb_live::serve(ctx, Box::pin(incoming), Box::pin(outgoing)).await;
    tracing::debug!(?end, "a live connection ended");
}
