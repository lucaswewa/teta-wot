//! HTTP binding of `teta-wot`, in two wire profiles.
//!
//! [`router`] turns a [`Runtime`] into an axum [`Router`]. By default it
//! behaves on the wire using teta-wot style; with
//! [`WireProfile::Wot`] it follows the W3C WoT HTTP Basic and SSE Profiles
//! where the two conflict.
//!
//! | Route | Methods |
//! |---|---|
//! | `{prefix}/{thing}/` | `GET` the Thing Description |
//! | `{prefix}/{thing}/{property}` | `GET`, and `PUT` if writable (201 and `null`; 204 in `wot`); `GET` with `Accept: text/event-stream` observes a data property (SSE) |
//! | `{prefix}/{thing}/{property}/reset` | `POST` if resettable and writable |
//! | `{prefix}/{thing}/{action}` | `POST` invokes (201 with `Location`; 200 or 204 for a synchronous action in `wot`), `GET` lists invocations |
//! | `{prefix}/{thing}/properties` | `GET` reads all properties, or observes them all with `Accept: text/event-stream`; `PUT` writes several (204) |
//! | `{prefix}/{thing}/actions` | `GET` the invocations of each action, newest first|
//! | `{prefix}/{thing}/events` | `GET` subscribes to all events (SSE) |
//! | `{prefix}/action_invocations` | `GET` lists every invocation |
//! | `{prefix}/action_invocations/{id}` | `GET` (`queryaction`), `DELETE` cancels (`cancelaction`) |
//! | `{prefix}/action_invocations/{id}/output` | `GET` |
//! | `{prefix}/things/`, `{prefix}/thing_descriptions/` | `GET` |
//! | `{prefix}/{thing}/{event}` | `GET` subscribes to an event (SSE) |
//! | `{prefix}/{thing}/ws` | a WebSocket, plus event subscriptions |
//! | `{prefix}/{thing}/{path}` | a custom [`Endpoint`]'s method |
//! | `/.well-known/wot`, `{prefix}/directory/things`, `{prefix}/directory/things/{id}` | `GET` and `HEAD`: discovery, a read-only TD Directory |
//! | `/openapi.json`, `/docs`, `/docs/oauth2-redirect`, `/redoc` | `GET` and `HEAD`: the OpenAPI document ([`openapi`]) and the docs pages, at the root as in FastAPI |
//!
//! In the `tetathing` profile, routing, redirects, 404/405, CORS, error
//! bodies and invocation JSON follow the reference as captured in
//! `conformance/fixtures/http/`. The W3C additions that don't conflict are
//! on in both profiles: the TD `id`, the `Location` header,
//! observation and events described in the TD, top-level forms, invocation forms and discovery.
//! The `wot` profile changes: 204 writes,
//! 400 validation errors, `application/problem+json`, and `ActionStatus`
//! objects.

mod cors;
mod discovery;
mod docs;
mod endpoint;
mod fallback;
mod handlers;
mod observe;
mod openapi;
mod output;
mod problem;
mod render;
mod routes;
mod security;

use std::sync::Arc;

use axum::Router;
use teta_wot_core::Runtime;
use uuid::Uuid;

pub use docs::{OFFLINE as DOCS_OFFLINE, REDOC_VERSION, SWAGGER_UI_VERSION};
pub use endpoint::Endpoint;
pub use fallback::{FallbackPage, fallback_router};
pub use openapi::{openapi, operation_id};
pub use problem::{PROBLEM_TYPES_URL, problem_type};
pub use routes::RESERVED_THING_NAMES;

/// The identifier of the W3C WoT HTTP Basic Profile, in the `profile` of
/// TDs served in the `wot` profile.
pub const HTTP_BASIC_PROFILE: &str = "https://www.w3.org/2022/wot/profile/http-basic/v1";

/// The identifier of the W3C WoT HTTP SSE Profile, in the `profile` of TDs
/// served in the `wot` profile.
pub const HTTP_SSE_PROFILE: &str = "https://www.w3.org/2022/wot/profile/http-sse/v1";

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
    /// The API's title in the OpenAPI document and the docs pages
    pub api_title: String,
    /// The API's version in the OpenAPI document.
    pub api_version: String,
    /// How the server behaves where tetathing and the W3C differ.
    pub profile: WireProfile,
    /// The credentials every interaction requires, if any.
    pub security: Option<Security>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            api_prefix: String::new(),
            server_id: default_server_id(),
            api_title: "teta-wot".to_owned(),
            api_version: "0.1.0".to_owned(),
            profile: WireProfile::default(),
            security: None,
        }
    }
}

/// The wire profile. It is server-wide.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WireProfile {
    /// teta-wot behaviour, which its clients depend on
    /// (the default).
    #[default]
    TetaThing,
    /// The W3C WoT HTTP Basic and SSE Profiles: 204 for writes, 400 and
    /// `application/problem+json` for errors, `ActionStatus` objects,
    /// synchronous actions and named server-sent events.
    Wot,
}

impl WireProfile {
    /// The profile's name in configuration files: `tetathing` or `wot`.
    pub fn as_str(self) -> &'static str {
        match self {
            WireProfile::TetaThing => "tetathing",
            WireProfile::Wot => "wot",
        }
    }

    /// The profile named `name`, if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "tetathing" => Some(WireProfile::TetaThing),
            "wot" => Some(WireProfile::Wot),
            _ => None,
        }
    }
}

impl std::fmt::Display for WireProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Credentials that every interaction with a Thing requires, described in
/// the TDs' `securityDefinitions`. Thing Descriptions,
/// the directory, the OpenAPI document and the docs stay public, so that
/// clients can learn what to send.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Security {
    /// HTTP Basic authentication (RFC 7617), as the WoT Profile allows.
    Basic {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// A bearer token (RFC 6750), compared as an opaque string.
    Bearer {
        /// The token.
        token: String,
    },
}

impl Security {
    /// HTTP Basic authentication with one user name and password.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Security::Basic {
            username: username.into(),
            password: password.into(),
        }
    }

    /// A bearer token.
    pub fn bearer(token: impl Into<String>) -> Self {
        Security::Bearer {
            token: token.into(),
        }
    }
}

// Credentials stay out of logs.
impl std::fmt::Debug for Security {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Security::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .finish_non_exhaustive(),
            Security::Bearer { .. } => f.debug_struct("Bearer").finish_non_exhaustive(),
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
    let openapi = serde_json::to_vec(&openapi::openapi(&runtime, &options))
        .unwrap_or_default()
        .into();
    let app = Arc::new(handlers::App {
        runtime,
        routes,
        options,
        openapi,
    });
    Ok(Router::new()
        .fallback(handlers::dispatch)
        .with_state(app)
        .layer(axum::middleware::from_fn(cors::cors)))
}

/// A Thing's Thing Description as the server serves it at `base` (such as
/// `http://localhost:5000/`): with that base, the TD `id`, and the forms
/// and links of observation, events and streams. With the OpenAPI document
/// ([`openapi`]), it lets TDs be written to files without a server.
pub fn thing_description(
    runtime: &Runtime,
    thing: &str,
    base: &str,
    options: &HttpOptions,
) -> Result<serde_json::Value, String> {
    let urls = render::Urls::from_base(base, &options.api_prefix);
    handlers::served_td(runtime, thing, &urls, options)
}

/// A stable TD `id` for a Thing: `urn:uuid:` followed by the UUIDv5
/// of `{server_id}/{thing}` in the `teta-wot` namespace, so it survives
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
