//! HTTP binding of `teta-wot`.
//!
//! [`router`] turns a [`Runtime`] into an axum [`Router`]
//!
//! | Route | Methods |
//! |---|---|
//! | `{prefix}/{thing}/` | `GET` the Thing Description |
//! | `{prefix}/{thing}/{property}` | `GET`, and `PUT` if writable (201, `null`) |
//! | `{prefix}/{thing}/{property}/reset` | `POST` if resettable and writable |
//! | `{prefix}/{thing}/{action}` | `POST` invokes (201 with `Location`), `GET` lists invocations |
//! | `{prefix}/action_invocations` | `GET` lists every invocation |
//! | `{prefix}/action_invocations/{id}` | `GET`, `DELETE` cancels |
//! | `{prefix}/action_invocations/{id}/output` | `GET` |
//! | `{prefix}/things/`, `{prefix}/thing_descriptions/` | `GET` |
//! | `{prefix}/{thing}/{event}` | `GET` subscribes to an event (SSE) |
//! | `{prefix}/{thing}/ws` | a WebSocket, plus event subscriptions |
//! | `{prefix}/{thing}/{path}` | a custom [`Endpoint`]'s method |
//!

mod cors;
mod endpoint;
mod fallback;
mod handlers;
mod observe;
mod render;
mod routes;

use std::sync::Arc;

use axum::Router;
use teta_wot_core::Runtime;
use uuid::Uuid;

pub use endpoint::Endpoint;
pub use fallback::{FallbackPage, fallback_router};
pub use routes::RESERVED_THING_NAMES;

// Custom endpoints are written with this axum; re-exported so that Thing
// code uses the same version.
#[doc(no_inline)]
pub use axum;

/// Options of the HTTP binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOptions {
    /// A prefix for every route, such as `/api/v1`, or empty.
    pub api_prefix: String,
    /// Identifies the server in TD `id`s (see [`td_id`]).
    pub server_id: String,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            api_prefix: String::new(),
            server_id: default_server_id(),
        }
    }
}

/// The HTTP binding can't be set up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RouteError {
    /// The API prefix doesn't match `^(/[\w-]+)*$`.
    #[error("invalid API prefix `{0}`: it must be empty or like `/api/v1`")]
    InvalidPrefix(String),
    /// A Thing's name clashes with one of the server's routes.
    #[error("the Thing name `{0}` is reserved for the server's own routes")]
    ReservedThingName(String),
    /// A custom endpoint answers a route that another route already answers.
    #[error("an endpoint of Thing `{thing}` answers `{route}`, which is already taken")]
    EndpointConflict {
        /// The Thing.
        thing: String,
        /// The method and path.
        route: String,
    },
    /// A custom endpoint wasn't made with [`Endpoint`].
    #[error("the endpoint `{path}` of Thing `{thing}` wasn't made with `wot_http::Endpoint`")]
    ForeignEndpoint {
        /// The Thing.
        thing: String,
        /// The endpoint's path.
        path: String,
    },
}

/// Builds the router for a runtime's Things. The runtime should be started
/// (and stopped) separately, for example by `wot-server`.
pub fn router(runtime: Arc<Runtime>, options: HttpOptions) -> Result<Router, RouteError> {
    let prefix_ok = options.api_prefix.is_empty()
        || options.api_prefix.split('/').skip(1).all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        }) && options.api_prefix.starts_with('/');
    if !prefix_ok {
        return Err(RouteError::InvalidPrefix(options.api_prefix));
    }
    let routes = routes::Routes::build(&runtime, &options.api_prefix)?;
    let app = Arc::new(handlers::App {
        runtime,
        routes,
        options,
    });
    Ok(Router::new()
        .fallback(handlers::dispatch)
        .with_state(app)
        .layer(axum::middleware::from_fn(cors::cors)))
}

/// A stable TD `id` for a Thing: `urn:uuid:` followed by the UUIDv5
/// of `{server_id}/{thing}` in the `wot-rs` namespace, so it survives
/// restarts and differs between servers.
pub fn td_id(server_id: &str, thing: &str) -> String {
    let namespace = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        b"https://github.com/lucaswewa/wot#td-id",
    );
    format!(
        "urn:uuid:{}",
        Uuid::new_v5(&namespace, format!("{server_id}/{thing}").as_bytes())
    )
}

/// The default server ID: the computer's name (`COMPUTERNAME`, or
/// `HOSTNAME`), or `localhost`.
pub fn default_server_id() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_owned())
}
