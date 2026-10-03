//! Web of Things support for the teta-wot workspace.
//!
//! This facade is the only crate applications depend on. The internal crates
//! are free to change shape behind it.
//!
//! So far it provides:
//!
//! - the runtime (re-exported from `wot-core`): Things defined with
//!   [`ThingDefinition`], property cells ([`Prop`]), actions
//!   ([`Action`]) with cancellation, invocations, the global lock, the
//!   message broker and device actors ([`Device`]), all run by a
//!   [`Runtime`];
//! - [`server`]: the HTTP server ([`server::ThingServer`]), which serves the
//!   runtime over HTTP, and shuts down gracefully;
//!   its OpenAPI document ([`server::openapi`]) with Swagger UI and ReDoc;
//!   Configuration files ([`server::ServerConfig`]) with a
//!   registry of Thing types ([`server::ThingRegistry`]), and
//!   command line ([`server::cli`]) with its fallback server;
//! - composition and persistence: slots ([`Slot`], [`OptSlot`], [`SlotMap`],
//!   and interfaces declared with [`interface`]), settings saved to disk,
//!   services ([`Dep`]) and the [`Server`] handle;
//! - observation and events: events ([`Event`]) emitted from async or
//!   synchronous code, and properties, action status and events observed
//!   over WebSocket (`/{thing}/ws`) and server-sent
//!   events, all described in the TD;
//! - binary data: Blobs ([`Blob`], with media types in [`blob`]) as action
//!   inputs and outputs, downloaded from `/blob/{id}`, and MJPEG streams
//!   ([`MjpegStream`]) fed from any thread and watched in a browser;
//! - [`td`]: the Thing Description model, its builders, and the conversion
//!   from Rust types to TD `DataSchema`s;
//! - with the `testing` feature, [`testing`]: helpers for testing Things,
//!   and an in-process HTTP test client;
//! - the authoring macros: [`derive@Thing`] for a Thing's struct and its
//!   field affordances, and [`thing_impl`] for its methods;
//! - [`http`]: custom endpoints ([`http::Endpoint`]) and the `axum` they are
//!   written with;
//! - the W3C WoT Profile, as the `wot` wire profile
//!   ([`server::WireProfile`]), discovery (`/.well-known/wot`, a TD
//!   Directory, and DNS-SD with the `mdns` feature), and optional
//!   credentials ([`server::Security`]);
//! - with the `ndarray` feature, `NdArray`: arrays as nested lists, with the
//!   `ndarray` crate re-exported as `teta_wot::ndarray`.
//!
//! `use teta_wot::prelude::*;` brings in what Thing code usually needs.

pub use teta_wot_core::*;
pub use teta_wot_macros::{Thing, interface, thing_impl};
pub use teta_wot_td as td;

/// The `ndarray` crate, which [`NdArray`] wraps (feature `ndarray`).
#[cfg(feature = "ndarray")]
pub use ndarray;

/// The HTTP server and binding.
pub mod server {
    pub use teta_wot_http::{
        DOCS_OFFLINE, FallbackPage, HTTP_BASIC_PROFILE, HTTP_SSE_PROFILE, HttpOptions,
        PROBLEM_TYPES_URL, REDOC_VERSION, RESERVED_THING_NAMES, RouteError, SWAGGER_UI_VERSION,
        Security, WireProfile, default_server_id, fallback_router, openapi, operation_id,
        problem_type, td_id, thing_description,
    };
    pub use teta_wot_server::{
        ConfigError, DEFAULT_SHUTDOWN_GRACE, RESERVED_CONFIG_THING_NAMES, SecurityConfig,
        ServeError, ServerBuildError, ServerConfig, ThingConfig, ThingRegistry, ThingServer,
        ThingServerBuilder, cli, normalise_class_name, shutdown_signal,
    };
    #[cfg(feature = "mdns")]
    pub use teta_wot_server::{DIRECTORY_SERVICE_TYPE, WOT_SERVICE_TYPE};
}

/// Custom HTTP endpoints: [`Endpoint`](http::Endpoint), and the
/// version of `axum` whose extractors and responses they use.
pub mod http {
    pub use teta_wot_http::Endpoint;
    pub use teta_wot_http::axum;
}

/// Helpers for tests: running action code in a fake invocation, and an
/// in-process HTTP client (`testing` feature).
#[cfg(feature = "testing")]
pub mod testing {
    pub use teta_wot_core::testing::*;
    pub use teta_wot_server::testing::*;
}

/// What Thing code usually needs.
pub mod prelude {
    #[cfg(feature = "ndarray")]
    pub use teta_wot_core::NdArray;
    pub use teta_wot_core::{
        Action, ActionCtx, ActionError, Blob, BoxFuture, CancelToken, Cancelled, Constraints,
        DataProperty, Dep, Device, DeviceError, DeviceOptions, Driver, Event, EventSpec,
        FromConfig, FunctionalProperty, InvocationStatus, MjpegStream, NoConfig, NoInput, OptSlot,
        Prop, PropertyError, Runtime, Server, Slot, SlotMap, Thing, ThingCtx, ThingDefinition,
        ThingRef, cancellable_sleep, check_cancelled,
    };
    pub use teta_wot_macros::{Thing, interface, thing_impl};
    pub use teta_wot_server::ThingServer;
}
