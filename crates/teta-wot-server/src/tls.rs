//! Serving over TLS (feature `tls`), with HTTP/2 for clients that negotiate
//! it by ALPN. Uses rustls with the `ring` crypto provider.
//!
//! ```no_run
//! use teta_wot_server::tls::{TlsListener, server_config};
//! use teta_wot_server::{ThingServer, shutdown_signal};
//!
//! # async fn run(server: ThingServer) -> anyhow::Result<()> {
//! let config = server_config(&std::fs::read("cert.pem")?, &std::fs::read("key.pem")?)?;
//! let tcp = tokio::net::TcpListener::bind("0.0.0.0:8443").await?;
//! server.serve_with_listener(TlsListener::new(tcp, config)?, shutdown_signal()).await?;
//! # Ok(())
//! # }
//! ```

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::serve::Listener;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::{self, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::server::TlsStream;

/// The rustls this module uses, for building a `ServerConfig` by hand.
pub use tokio_rustls::rustls;

/// How long a client has to complete its TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A certificate or key can't be used.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// The certificate PEM has no certificate.
    #[error("the PEM has no certificate")]
    NoCertificate,
    /// A PEM can't be read.
    #[error("can't read the PEM: {0}")]
    Pem(#[from] pem::Error),
    /// rustls refused the certificate or key.
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
}

/// A server's TLS configuration from its certificate chain and private key,
/// both PEM. It offers HTTP/2, then HTTP/1.1, by ALPN.
pub fn server_config(
    certificate_chain: &[u8],
    private_key: &[u8],
) -> Result<rustls::ServerConfig, TlsError> {
    let chain = CertificateDer::pem_slice_iter(certificate_chain).collect::<Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        return Err(TlsError::NoCertificate);
    }
    let key = PrivateKeyDer::from_pem_slice(private_key)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// A listener that accepts TCP connections and completes their TLS
/// handshakes, each in its own task, so that a slow or failing client
/// never holds up the others. Serve it with
/// [`ThingServer::serve_with_listener`](crate::ThingServer::serve_with_listener).
///
/// Clients that negotiate HTTP/2 by ALPN get HTTP/2; WebSockets use
/// HTTP/1.1.
#[derive(Debug)]
pub struct TlsListener {
    connections: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local_addr: SocketAddr,
    acceptor: JoinHandle<()>,
}

impl TlsListener {
    /// Accepts connections on `listener`, with `config` (see
    /// [`server_config`]).
    pub fn new(listener: TcpListener, config: rustls::ServerConfig) -> io::Result<Self> {
        let local_addr = listener.local_addr()?;
        let (send, connections) = mpsc::channel(64);
        let tls = TlsAcceptor::from(Arc::new(config));
        let acceptor = tokio::spawn(accept_all(listener, tls, send));
        Ok(Self {
            connections,
            local_addr,
            acceptor,
        })
    }
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        // Stops accepting, which closes the port.
        self.acceptor.abort();
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.connections.recv().await {
            Some(connection) => connection,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

/// Accepts connections for ever, handing each one, once its handshake has
/// completed, to `send`.
async fn accept_all(
    listener: TcpListener,
    tls: TlsAcceptor,
    send: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    loop {
        let (tcp, addr) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!("accepting a connection failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let (tls, send) = (tls.clone(), send.clone());
        tokio::spawn(async move {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, tls.accept(tcp)).await {
                Ok(Ok(stream)) => {
                    let _ = send.send((stream, addr)).await;
                }
                Ok(Err(error)) => tracing::debug!(%addr, "TLS handshake failed: {error}"),
                Err(_) => tracing::debug!(%addr, "TLS handshake timed out"),
            }
        });
    }
}
