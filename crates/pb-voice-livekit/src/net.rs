//! LiveKit's network transport (the signalling websocket and its few HTTP calls) on our own TLS (pb-tls): the system's
//! certificate roots plus Mozilla's, so a self-hosted LiveKit with a private CA works, and no second TLS stack (the
//! SDK's built-in transport brings `ring`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use livekit_net::{
    Header, HttpClient, HttpMethod, HttpResponse, TransportError, WsClient, WsConnectResult, WsConnection,
};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

/// How long one of LiveKit's HTTP calls may take.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Registers the transport for the whole process (once; later calls return the first outcome).
pub fn install() -> Result<(), String> {
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let net = Arc::new(Net::new()?);
            livekit_net::set_ws_client(net.clone());
            livekit_net::set_http_client(net);
            Ok(())
        })
        .clone()
}

struct Net {
    tls: Connector,
    http: reqwest::Client,
}

impl Net {
    fn new() -> Result<Net, String> {
        let tls = pb_tls::client_config().map_err(|e| e.to_string())?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured((*tls).clone())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Net {
            tls: Connector::Rustls(tls),
            http,
        })
    }
}

fn header(h: &Header) -> Result<(HeaderName, HeaderValue), TransportError> {
    let name = HeaderName::from_bytes(h.name.as_bytes()).map_err(|e| TransportError::Other(e.to_string()))?;
    let mut value = HeaderValue::from_str(&h.value).map_err(|e| TransportError::Other(e.to_string()))?;
    // The signalling connection authenticates with a bearer token.
    value.set_sensitive(true);
    Ok((name, value))
}

#[async_trait::async_trait]
impl WsClient for Net {
    async fn connect(
        &self,
        url: String,
        headers: Vec<Header>,
        timeout_ms: u64,
    ) -> Result<WsConnectResult, TransportError> {
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| TransportError::Connection(e.to_string()))?;
        for h in &headers {
            let (name, value) = header(h)?;
            request.headers_mut().insert(name, value);
        }
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(self.tls.clone()));
        let (ws, _) = tokio::time::timeout(Duration::from_millis(timeout_ms), connect)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| match e {
                // An HTTP answer to the upgrade (403, 404 …) is told apart from a network error.
                WsError::Http(resp) => TransportError::Http {
                    status: resp.status().as_u16(),
                },
                other => TransportError::Connection(other.to_string()),
            })?;
        let (writer, reader) = ws.split();
        Ok(WsConnectResult {
            connection: Arc::new(Connection {
                writer: Mutex::new(writer),
                reader: Mutex::new(reader),
            }),
        })
    }
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Connection {
    writer: Mutex<SplitSink<Ws, Message>>,
    reader: Mutex<SplitStream<Ws>>,
}

#[async_trait::async_trait]
impl WsConnection for Connection {
    async fn send(&self, frame: Vec<u8>) -> Result<(), TransportError> {
        self.writer
            .lock()
            .await
            .send(Message::Binary(frame.into()))
            .await
            .map_err(|e| TransportError::Connection(e.to_string()))
    }

    async fn recv(&self) -> Result<Option<Vec<u8>>, TransportError> {
        let mut reader = self.reader.lock().await;
        loop {
            match reader.next().await {
                Some(Ok(Message::Binary(data))) => return Ok(Some(data.to_vec())),
                Some(Ok(Message::Ping(payload))) => {
                    let _ = self.writer.lock().await.send(Message::Pong(payload)).await;
                }
                // Signalling sends only binary frames.
                Some(Ok(Message::Pong(_) | Message::Frame(_) | Message::Text(_))) => {}
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                // A peer that hangs up without the closing handshake has closed all the same.
                Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => return Ok(None),
                Some(Err(WsError::Protocol(
                    tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                ))) => return Ok(None),
                Some(Err(WsError::Io(e))) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Some(Err(e)) => return Err(TransportError::Connection(e.to_string())),
            }
        }
    }

    async fn close(&self) {
        let _ = self.writer.lock().await.close().await;
    }
}

#[async_trait::async_trait]
impl HttpClient for Net {
    async fn request(
        &self,
        method: HttpMethod,
        url: String,
        headers: Vec<Header>,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, TransportError> {
        let mut req = match method {
            HttpMethod::Get => self.http.get(&url),
            HttpMethod::Post => self.http.post(&url),
        };
        for h in &headers {
            let (name, value) = header(h)?;
            req = req.header(name.as_str(), value.as_bytes());
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let res = req.send().await.map_err(|e| {
            if e.is_timeout() {
                TransportError::Timeout
            } else {
                TransportError::Connection(e.to_string())
            }
        })?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .filter_map(|(n, v)| {
                v.to_str().ok().map(|v| Header {
                    name: n.as_str().to_owned(),
                    value: v.to_owned(),
                })
            })
            .collect();
        let body = res
            .bytes()
            .await
            .map_err(|e| TransportError::Other(e.to_string()))?
            .to_vec();
        Ok(HttpResponse { status, headers, body })
    }
}
