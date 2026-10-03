//! Custom endpoints: axum handlers
//! served at `/{thing}/{path}`, outside the Thing Description.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::Request;
use axum::handler::Handler;
use axum::response::Response;
use teta_wot_core::{EndpointHandler, EndpointSpec};
use tower::util::BoxCloneSyncService;

/// The type-erased service behind an endpoint, as the core carries it.
pub(crate) type EndpointService = BoxCloneSyncService<Request, Response, Infallible>;

/// A custom endpoint of a Thing type: an axum handler made for each Thing
/// instance. `#[endpoint]` methods generate these; with the builder API,
/// pass one to `ThingDefinition::endpoint`.
///
/// ```
/// use std::sync::Arc;
/// use teta_wot_core::{Thing, ThingDefinition};
/// use teta_wot_http::Endpoint;
///
/// struct Greeter;
///
/// impl Thing for Greeter {
///     fn definition() -> ThingDefinition<Self> {
///         ThingDefinition::new("Greeter").endpoint(Endpoint::new("get", "hello.txt", |_: Arc<Greeter>| {
///             || async { "Hello!" }
///         }))
///     }
/// }
/// ```
pub struct Endpoint<T> {
    spec: EndpointSpec<T>,
}

impl<T: Send + Sync + 'static> Endpoint<T> {
    /// An endpoint answering `method` at `/{thing}/{path}`. `handler` makes
    /// the axum handler for one Thing instance; its arguments are extractors
    /// and its output anything that implements `IntoResponse`.
    pub fn new<F, H, M>(method: &str, path: &str, handler: F) -> Self
    where
        F: FnOnce(Arc<T>) -> H + Send + 'static,
        H: Handler<M, ()>,
        M: 'static,
    {
        let build = move |thing: &Arc<T>| {
            let service: EndpointService =
                BoxCloneSyncService::new(handler(Arc::clone(thing)).with_state(()));
            Arc::new(service) as EndpointHandler
        };
        Self {
            spec: EndpointSpec::new(method, path, build),
        }
    }

    /// Sets the description, shown in the OpenAPI document.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.spec = self.spec.description(description);
        self
    }

    /// Sets the name its OpenAPI `operationId` is made from: see
    /// [`EndpointSpec::name`].
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.spec = self.spec.name(name);
        self
    }

    /// Lists the endpoint in the TD's `links`: see
    /// [`EndpointSpec::link`].
    #[must_use]
    pub fn link(mut self, rel: impl Into<String>, media_type: Option<&str>) -> Self {
        self.spec = self.spec.link(rel, media_type);
        self
    }
}

impl<T> From<Endpoint<T>> for EndpointSpec<T> {
    fn from(endpoint: Endpoint<T>) -> Self {
        endpoint.spec
    }
}
