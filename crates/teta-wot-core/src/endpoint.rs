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
    pub(crate) link: Option<EndpointLink>,
    pub(crate) name: Option<String>,
    pub(crate) websocket: bool,
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
    ///
    /// The path may have parameters, whole segments such as
    /// `tiles/{level}/{tile}`, and end with a wildcard, `files/{*rest}`,
    /// which matches the rest of the path. A literal segment beats a
    /// parameter, and a parameter beats a wildcard.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        build: impl FnOnce(&Arc<T>) -> EndpointHandler + Send + 'static,
    ) -> Self {
        Self {
            method: method.into().to_ascii_uppercase(),
            path: path.into(),
            description: None,
            link: None,
            name: None,
            websocket: false,
            build: Box::new(build),
        }
    }

    /// Sets the description, shown in the OpenAPI document.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the name its OpenAPI `operationId` is made from. `#[endpoint]` uses the
    /// method's name; without one, the path is used.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Lists the endpoint in the Thing Description's `links`,
    /// with a relation type (such as `related` or `alternate`) and,
    /// optionally, the media type it returns. Endpoints aren't listed
    /// otherwise.
    #[must_use]
    pub fn link(mut self, rel: impl Into<String>, media_type: Option<&str>) -> Self {
        self.link = Some(EndpointLink {
            rel: rel.into(),
            media_type: media_type.map(str::to_owned),
            members: serde_json::Map::new(),
        });
        self
    }

    /// Marks the endpoint as a WebSocket that, when its upgrade request
    /// carries no credentials, authenticates with its first message.
    /// Browsers can't add headers to WebSockets. The handler must then do
    /// that check itself, with `teta_wot::http::FirstMessageAuth`; binary
    /// streams do.
    #[must_use]
    pub fn websocket(mut self) -> Self {
        self.websocket = true;
        self
    }

    /// Adds a member to the endpoint's TD link, such as a binary stream's
    /// `subprotocol`. It does nothing unless [`link`](Self::link) was called
    /// first.
    #[must_use]
    pub fn link_member(mut self, name: impl Into<String>, value: serde_json::Value) -> Self {
        if let Some(link) = &mut self.link {
            link.members.insert(name.into(), value);
        }
        self
    }
}

/// How an endpoint is listed in the TD's `links`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointLink {
    /// The relation type.
    pub rel: String,
    /// The media type of what it returns, if given.
    pub media_type: Option<String>,
    /// Other members of the link.
    pub members: serde_json::Map<String, serde_json::Value>,
}

/// An endpoint of a running Thing.
#[derive(Clone)]
pub struct EndpointEntry {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) description: Option<String>,
    pub(crate) link: Option<EndpointLink>,
    pub(crate) name: Option<String>,
    pub(crate) websocket: bool,
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

    /// The name its OpenAPI `operationId` is made from: the one given, or
    /// the path.
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.path)
    }

    /// How it is listed in the TD's `links`, if it is.
    pub fn link(&self) -> Option<&EndpointLink> {
        self.link.as_ref()
    }

    /// Whether it is a WebSocket that authenticates its first message.
    pub fn is_websocket(&self) -> bool {
        self.websocket
    }

    /// The handler, for the HTTP binding to downcast.
    pub fn handler(&self) -> &EndpointHandler {
        &self.handler
    }
}

/// Why an endpoint path can't be used, if it can't. It must be relative (no
/// leading `/`), with non-empty segments that aren't `.` or `..`. A segment
/// may be a path parameter, `{name}`, and the last one a wildcard,
/// `{*name}`. Names are letters, digits and `_`, not starting with a digit,
/// and are used once.
pub(crate) fn path_problem(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') {
        return Some(format!(
            "endpoint path `{path}` must be relative to the Thing, without a leading `/`"
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    let mut names = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return Some(format!(
                "endpoint path `{path}` has an empty, `.` or `..` segment"
            ));
        }
        if !segment.contains(['{', '}']) {
            continue;
        }
        let Some(inner) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
            return Some(format!(
                "endpoint path `{path}`: a parameter must be a whole segment, \
                 such as `{{name}}` or `{{*rest}}`, not `{segment}`"
            ));
        };
        let (name, wildcard) = inner
            .strip_prefix('*')
            .map_or((inner, false), |name| (name, true));
        let mut chars = name.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Some(format!(
                "endpoint path `{path}`: `{segment}` needs a name of letters, digits \
                 and `_`, not starting with a digit"
            ));
        }
        if wildcard && index + 1 != segments.len() {
            return Some(format!(
                "endpoint path `{path}`: the wildcard `{segment}` must be the last segment"
            ));
        }
        if names.contains(&name) {
            return Some(format!(
                "endpoint path `{path}` uses the parameter `{name}` twice"
            ));
        }
        names.push(name);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::path_problem;

    #[test]
    fn paths_with_parameters_and_a_final_wildcard_are_accepted() {
        for path in [
            "a.txt",
            "tiles/{level}/{tile}",
            "files/{*rest}",
            "{_x1}/{*y}",
        ] {
            assert_eq!(path_problem(path), None, "{path}");
        }
    }

    #[test]
    fn malformed_paths_are_explained() {
        for (path, problem) in [
            ("/a", "leading `/`"),
            ("a//b", "empty"),
            ("a/../b", "`..`"),
            ("{dataset}_files", "whole segment"),
            ("a/{b", "whole segment"),
            ("a/{}", "needs a name"),
            ("a/{1st}", "needs a name"),
            ("{*rest}/more", "last segment"),
            ("{id}/{id}", "twice"),
        ] {
            let found = path_problem(path).unwrap_or_default();
            assert!(found.contains(problem), "{path}: {found}");
        }
    }
}
