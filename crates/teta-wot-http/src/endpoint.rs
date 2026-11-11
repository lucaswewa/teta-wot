//! Custom endpoints: axum handlers
//! served at `/{thing}/{path}`, outside the Thing Description.

use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{FromRequestParts, Request};
use axum::handler::Handler;
use axum::response::Response;
use http::request::Parts;
use percent_encoding::percent_decode_str;
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
    pub(crate) spec: EndpointSpec<T>,
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

/// An extractor: the path parameters of a request to a custom endpoint, by
/// name, percent-decoded.
///
/// For an endpoint at `tiles/{level}/{tile}`, a request for
/// `/camera/tiles/3/0_1.jpeg` has `level` = `3` and `tile` = `0_1.jpeg`. A
/// wildcard, `files/{*rest}`, has the rest of the path, slashes included.
/// axum's own `Path` extractor doesn't work in endpoints, because `teta-wot`
/// routes requests itself.
///
/// ```
/// use teta_wot_http::PathParams;
///
/// async fn tile(params: PathParams) -> String {
///     let level: u32 = params.parse("level").unwrap_or(0);
///     format!("tile {} at level {level}", params.get("tile").unwrap_or_default())
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathParams(Vec<(String, String)>);

impl PathParams {
    /// Decodes the values as they are in the path, or `None` if one isn't
    /// UTF-8 once decoded.
    pub(crate) fn decode(raw: Vec<(String, String)>) -> Option<Self> {
        raw.into_iter()
            .map(|(name, value)| {
                let value = percent_decode_str(&value).decode_utf8().ok()?;
                Some((name, value.into_owned()))
            })
            .collect::<Option<_>>()
            .map(Self)
    }

    /// The value of the parameter `name`, if the endpoint's path has one.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.as_str())
    }

    /// The value of the parameter `name`, parsed, or `None` if there is no
    /// such parameter or it doesn't parse.
    pub fn parse<V: FromStr>(&self, name: &str) -> Option<V> {
        self.get(name)?.parse().ok()
    }

    /// Every parameter, as `(name, value)`, in the order of the path.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for PathParams {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().cloned().unwrap_or_default())
    }
}
