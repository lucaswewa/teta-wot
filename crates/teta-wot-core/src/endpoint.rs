//! Custom endpoints: HTTP routes of a Thing
//! that aren't affordances.
//!
//! The core doesn't know HTTP. An endpoint is a method, a path relative to
//! the Thing, and a handler that the HTTP binding made and knows how to
//! call; here it is only carried, as `dyn Any`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// The handler of an endpoint, bound to one Thing instance. Only the HTTP
/// binding that made it knows its type.
pub type EndpointHandler = Arc<dyn Any + Send + Sync>;

type BuildFn<T> = Box<dyn FnOnce(&Arc<T>) -> EndpointHandler + Send>;

/// An endpoint of a Thing type, ready to add to a
/// [`ThingDefinition`](crate::ThingDefinition).
pub struct EndpointSpec<T> {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) description: Option<String>,
    pub(crate) build: BuildFn<T>,
}

impl<T> fmt::Debug for EndpointSpec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointSpec")
            .field("method", &self.method)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl<T> EndpointSpec<T> {
    /// An endpoint answering `method` (upper case, such as `GET`) at
    /// `/{thing}/{path}`. `build` makes its handler for one Thing instance.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        build: impl FnOnce(&Arc<T>) -> EndpointHandler + Send + 'static,
    ) -> Self {
        Self {
            method: method.into().to_ascii_uppercase(),
            path: path.into(),
            description: None,
            build: Box::new(build),
        }
    }

    /// Sets the description (for the OpenAPI document of Phase 8).
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// An endpoint of a running Thing.
#[derive(Clone)]
pub struct EndpointEntry {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) description: Option<String>,
    pub(crate) handler: EndpointHandler,
}

impl fmt::Debug for EndpointEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointEntry")
            .field("method", &self.method)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl EndpointEntry {
    /// The HTTP method, upper case.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The path relative to the Thing, without a leading slash.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The handler, for the HTTP binding to downcast.
    pub fn handler(&self) -> &EndpointHandler {
        &self.handler
    }
}

/// Why an endpoint path can't be used, if it can't: it must be relative
/// (no leading `/`), with non-empty segments that aren't `.` or `..`.
pub(crate) fn path_problem(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') {
        return Some(format!(
            "endpoint path `{path}` must be relative to the Thing, without a leading `/`"
        ));
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Some(format!(
            "endpoint path `{path}` has an empty, `.` or `..` segment"
        ));
    }
    None
}
