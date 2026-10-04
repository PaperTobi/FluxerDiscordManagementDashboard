//! HTTPS for the web UI: a listener that hands axum connections once their TLS handshake is done. Handshakes run side
//! by side, so a slow or broken client never holds up the others.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::server::TlsStream;

/// A client gets this long for its handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// The address a request came from (over HTTP or HTTPS).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Peer(pub SocketAddr);

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TcpListener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, TcpListener>) -> Peer {
        Peer(*stream.remote_addr())
    }
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TlsListener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, TlsListener>) -> Peer {
        Peer(*stream.remote_addr())
    }
}

pub(crate) struct TlsListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    handshakes: JoinSet<Option<(TlsStream<TcpStream>, SocketAddr)>>,
}

impl TlsListener {
    pub(crate) fn new(tcp: TcpListener, config: Arc<ServerConfig>) -> TlsListener {
        TlsListener {
            tcp,
            acceptor: TlsAcceptor::from(config),
            handshakes: JoinSet::new(),
        }
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            tokio::select! {
                tcp = self.tcp.accept() => match tcp {
                    Ok((stream, addr)) => {
                        let acceptor = self.acceptor.clone();
                        self.handshakes.spawn(async move {
                            match tokio::time::timeout(HANDSHAKE, acceptor.accept(stream)).await {
                                Ok(Ok(tls)) => Some((tls, addr)),
                                Ok(Err(e)) => {
                                    tracing::debug!(%addr, error = %e, "a TLS handshake failed");
                                    None
                                }
                                Err(_) => None,
                            }
                        });
                    }
                    // Out of file handles and the like: wait a little rather than spin.
                    Err(e) => {
                        tracing::warn!(error = %e, "accepting a connection failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                Some(done) = self.handshakes.join_next(), if !self.handshakes.is_empty() => {
                    if let Ok(Some(conn)) = done {
                        return conn;
                    }
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}
