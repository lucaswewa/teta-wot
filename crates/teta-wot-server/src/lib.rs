//! The `teta-wot` server: Things, the HTTP binding, and their lifecycle.
//!
//! [`ThingServer`] puts a [`Runtime`] behind the HTTP binding and runs it:
//!
//! 1. **create:** the builder instantiates each Thing's definition and the
//!    routes;
//! 2. **start:** the Things start in order: their devices open, then
//!    [`Thing::start`] runs. A failure stops what has started, is logged,
//!    and is returned as [`ServeError::Startup`];
//! 3. **serve** until the shutdown signal;
//! 4. **shut down**: stop accepting connections and let requests in
//!    progress finish, cancel unfinished invocations, wait up to the grace
//!    period for both, then stop the Things in reverse order and close their
//!    devices.
//!
//! [`shutdown_signal`] is the default signal.

use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use teta_wot_core::{BuildError, FromConfig, Runtime, RuntimeBuilder, StartupError, Thing};
use teta_wot_http::{HttpOptions, RouteError};
use tokio::net::TcpListener;

#[cfg(feature = "testing")]
pub mod testing;

/// The default time to wait for requests and invocations to finish on shutdown.
pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// A server can't be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerBuildError {
    /// A Thing or its definition is invalid.
    #[error(transparent)]
    Runtime(#[from] BuildError),
    /// The routes can't be built.
    #[error(transparent)]
    Routes(#[from] RouteError),
}

/// A server failed while running.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServeError {
    /// A Thing failed to start.
    #[error(transparent)]
    Startup(#[from] StartupError),
    /// Binding or serving failed.
    #[error("the server failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Builds a [`ThingServer`].
#[must_use]
pub struct ThingServerBuilder {
    runtime: RuntimeBuilder,
    http: HttpOptions,
    grace: Duration,
}

impl ThingServerBuilder {
    /// Adds a Thing under `name`.
    pub fn thing<T: Thing>(mut self, name: impl Into<String>, thing: T) -> Self {
        self.runtime = self.runtime.thing(name, thing);
        self
    }

    /// Adds a Thing that is already shared.
    pub fn thing_arc<T: Thing>(mut self, name: impl Into<String>, thing: Arc<T>) -> Self {
        self.runtime = self.runtime.thing_arc(name, thing);
        self
    }

    /// Adds a Thing built from its configuration (its `kwargs`), which is
    /// deserialised into `T::Config` when the server is built.
    pub fn thing_from_config<T: FromConfig>(
        mut self,
        name: impl Into<String>,
        kwargs: serde_json::Value,
    ) -> Self {
        self.runtime = self.runtime.thing_from_config::<T>(name, kwargs);
        self
    }

    /// Serves every route under a prefix such as `/api/v1`.
    pub fn api_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.http.api_prefix = prefix.into();
        self
    }

    /// Enables the global lock (off by default).
    pub fn global_lock(mut self, enabled: bool) -> Self {
        self.runtime = self.runtime.global_lock(enabled);
        self
    }

    /// The server ID used in TD `id`s (by default the computer's name).
    pub fn server_id(mut self, id: impl Into<String>) -> Self {
        self.http.server_id = id.into();
        self
    }

    /// How long shutdown waits for requests and invocations (5 s by default).
    pub fn shutdown_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Builds the Things and the routes.
    pub fn build(self) -> Result<ThingServer, ServerBuildError> {
        let runtime = Arc::new(self.runtime.build()?);
        let router = teta_wot_http::router(Arc::clone(&runtime), self.http)?;
        Ok(ThingServer {
            runtime,
            router,
            grace: self.grace,
        })
    }
}

/// Things served over HTTP.
pub struct ThingServer {
    runtime: Arc<Runtime>,
    router: Router,
    grace: Duration,
}

impl std::fmt::Debug for ThingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThingServer")
            .field("runtime", &self.runtime)
            .field("grace", &self.grace)
            .finish_non_exhaustive()
    }
}

impl ThingServer {
    /// Starts building a server.
    pub fn builder() -> ThingServerBuilder {
        ThingServerBuilder {
            runtime: Runtime::builder(),
            http: HttpOptions::default(),
            grace: DEFAULT_SHUTDOWN_GRACE,
        }
    }

    /// The runtime: Things, invocations, broker, lock.
    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    /// The HTTP routes, as an axum router.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// The shutdown grace period.
    pub fn shutdown_grace(&self) -> Duration {
        self.grace
    }

    /// Binds `addr` and serves until [`shutdown_signal`].
    pub async fn serve(self, addr: SocketAddr) -> Result<(), ServeError> {
        let listener = TcpListener::bind(addr).await?;
        self.serve_with(listener, shutdown_signal()).await
    }

    /// Starts the Things, serves on `listener` until `shutdown` completes,
    /// then shuts down (see the crate documentation).
    pub async fn serve_with(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), ServeError> {
        if let Err(failure) = self.runtime.start().await {
            tracing::error!(thing = %failure.thing, "{failure}");
            return Err(failure.into());
        }
        if let Ok(addr) = listener.local_addr() {
            tracing::info!(
                "serving {} Things on http://{addr}",
                self.runtime.things().count()
            );
        }

        let (stop, stopped) = tokio::sync::watch::channel(false);
        let serve = axum::serve(listener, self.router).with_graceful_shutdown(async move {
            let mut stopped = stopped;
            let _ = stopped.wait_for(|stop| *stop).await;
        });
        let mut server = tokio::spawn(serve.into_future());

        let result = tokio::select! {
            result = &mut server => Some(result),
            () = shutdown => None,
        };
        if let Some(result) = result {
            // The server stopped by itself: an I/O error.
            self.runtime.shutdown(self.grace).await;
            return match result {
                Ok(served) => served.map_err(ServeError::Io),
                Err(join) => Err(ServeError::Io(std::io::Error::other(join))),
            };
        }

        tracing::info!("shutting down");
        let _ = stop.send(true);
        self.runtime.invocations().cancel_all();
        let drained = tokio::time::timeout(self.grace, async {
            let _ = (&mut server).await;
            self.runtime.invocations().wait_idle().await;
        })
        .await;
        if drained.is_err() {
            tracing::warn!(
                "requests or invocations were still running after {:?}",
                self.grace
            );
            server.abort();
        }
        self.runtime.stop().await;
        tracing::info!("stopped");
        Ok(())
    }
}

/// Completes when the process is asked to stop: Ctrl-C, Ctrl-Break, the
/// console window closing, log-off or system shutdown on Windows; Ctrl-C
/// elsewhere.
pub async fn shutdown_signal() {
    #[cfg(windows)]
    {
        use tokio::signal::windows;

        // Waits for a console event, or forever if its handler couldn't be installed.
        macro_rules! event {
            ($install:expr) => {
                async {
                    match $install {
                        Ok(mut signal) => {
                            signal.recv().await;
                        }
                        Err(_) => std::future::pending::<()>().await,
                    }
                }
            };
        }

        tokio::select! {
            () = event!(windows::ctrl_c()) => tracing::info!("received Ctrl-C"),
            () = event!(windows::ctrl_break()) => tracing::info!("received Ctrl-Break"),
            () = event!(windows::ctrl_close()) => tracing::info!("the console is closing"),
            () = event!(windows::ctrl_logoff()) => tracing::info!("the user is logging off"),
            () = event!(windows::ctrl_shutdown()) => tracing::info!("the system is shutting down"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received Ctrl-C");
    }
}
