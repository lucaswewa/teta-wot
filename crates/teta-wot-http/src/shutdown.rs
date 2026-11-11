//! The server's shutdown, for handlers that hold connections open.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use http::request::Parts;
use tokio::sync::watch;

/// Completes when the server starts shutting down.
///
/// A handler that holds a connection open (a WebSocket, a stream, a long
/// poll) watches it and ends its response, so that shutdown doesn't wait for
/// the whole grace period. It is an extractor in merged routes and custom
/// endpoints. Outside a server, it never completes. Clones share one state.
///
/// ```
/// use teta_wot_http::ShutdownToken;
///
/// async fn wait(shutdown: ShutdownToken) -> &'static str {
///     shutdown.cancelled().await;
///     "the server is shutting down"
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ShutdownToken(Arc<watch::Sender<bool>>);

impl Default for ShutdownToken {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}

impl ShutdownToken {
    /// A token that hasn't been triggered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the shutdown: every [`cancelled`](Self::cancelled) completes.
    /// The server calls it when it starts to shut down.
    pub fn shut_down(&self) {
        self.0.send_replace(true);
    }

    /// Whether the shutdown has started.
    pub fn is_shutting_down(&self) -> bool {
        *self.0.borrow()
    }

    /// Completes when the shutdown starts, or at once if it has.
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|stopping| *stopping).await;
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ShutdownToken {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().cloned().unwrap_or_default())
    }
}
