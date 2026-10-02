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
//! - [`td`]: the Thing Description model, its builders, and the conversion
//!   from Rust types to TD `DataSchema`s;
//! - with the `testing` feature, [`testing`]: helpers for testing Things,
//!   and an in-process HTTP test client;
//! - the authoring macros: [`derive@Thing`] for a Thing's struct and its
//!   field affordances, and [`thing_impl`] for its methods;
//! - [`http`]: custom endpoints ([`http::Endpoint`]) and the `axum` they are
//!   written with.
//!
//! `use teta_wot::prelude::*;` brings in what Thing code usually needs.

pub use teta_wot_core::*;
pub use teta_wot_macros::{Thing, thing_impl};
pub use teta_wot_td as td;

/// The HTTP server and binding.
pub mod server {
    pub use teta_wot_http::{
        HttpOptions, RESERVED_THING_NAMES, RouteError, default_server_id, td_id,
    };
    pub use teta_wot_server::{
        DEFAULT_SHUTDOWN_GRACE, ServeError, ServerBuildError, ThingServer, ThingServerBuilder,
        shutdown_signal,
    };
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
    pub use teta_wot_core::{
        Action, ActionCtx, ActionError, CancelToken, Cancelled, Constraints, DataProperty, Device,
        DeviceError, DeviceOptions, Driver, FromConfig, FunctionalProperty, InvocationStatus,
        NoConfig, NoInput, Prop, PropertyError, Runtime, Thing, ThingCtx, ThingDefinition,
        ThingRef, cancellable_sleep, check_cancelled,
    };
    pub use teta_wot_macros::{Thing, thing_impl};
    pub use teta_wot_server::ThingServer;
}
