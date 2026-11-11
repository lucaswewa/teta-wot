//! The route table.

use http::Method;
use teta_wot_core::Runtime;

use crate::RouteError;
use crate::docs;
use crate::endpoint::EndpointService;

/// What a route does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Endpoint {
    ThingDescription {
        thing: String,
    },
    ReadProperty {
        thing: String,
        property: String,
    },
    WriteProperty {
        thing: String,
        property: String,
    },
    ResetProperty {
        thing: String,
        property: String,
    },
    InvokeAction {
        thing: String,
        action: String,
    },
    ListActionInvocations {
        thing: String,
        action: String,
    },
    /// `readallproperties`, or `observeallproperties` with server-sent
    /// events.
    ReadAllProperties {
        thing: String,
    },
    /// `writemultipleproperties`.
    WriteMultipleProperties {
        thing: String,
    },
    /// `queryallactions`.
    QueryAllActions {
        thing: String,
    },
    /// `subscribeallevents`.
    SubscribeAllEvents {
        thing: String,
    },
    /// `GET /.well-known/wot`: the directory's TD.
    WellKnown,
    /// `GET {prefix}/directory/things`: the TDs.
    DirectoryThings,
    /// `GET {prefix}/directory/things/{id}`: one TD.
    DirectoryThing,
    /// Server-sent events of an event affordance.
    SubscribeEvent {
        thing: String,
        event: String,
    },
    ListInvocations,
    /// `GET /openapi.json`.
    OpenApi,
    /// `GET /docs`.
    SwaggerUi,
    /// `GET /docs/oauth2-redirect`.
    OAuth2Redirect,
    /// `GET /redoc`.
    Redoc,
    /// A compiled-in docs asset: its index in `docs::ASSETS`.
    DocsAsset(usize),
    /// `GET {prefix}/blob/{id}`.
    DownloadBlob,
    /// An MJPEG stream.
    Stream {
        thing: String,
        stream: String,
    },
    /// An MJPEG stream's viewer page.
    StreamViewer {
        thing: String,
        stream: String,
    },
    GetInvocation,
    CancelInvocation,
    InvocationOutput,
    ThingPaths,
    ThingDescriptions,
    /// A custom endpoint: the index of its service in [`Routes::services`].
    Custom(usize),
}

impl Endpoint {
    /// Whether the route describes rather than interacts, and so needs no
    /// credentials: TDs, discovery, and the docs.
    pub(crate) fn is_public(&self) -> bool {
        matches!(
            self,
            Endpoint::ThingDescription { .. }
                | Endpoint::ThingDescriptions
                | Endpoint::ThingPaths
                | Endpoint::OpenApi
                | Endpoint::SwaggerUi
                | Endpoint::OAuth2Redirect
                | Endpoint::Redoc
                | Endpoint::DocsAsset(_)
                | Endpoint::WellKnown
                | Endpoint::DirectoryThings
                | Endpoint::DirectoryThing
        )
    }

    /// Whether the route is discovery's, whose errors are always problem
    /// details.
    pub(crate) fn is_discovery(&self) -> bool {
        matches!(
            self,
            Endpoint::WellKnown | Endpoint::DirectoryThings | Endpoint::DirectoryThing
        )
    }
}

/// A segment of a route's pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    Literal(String),
    /// A path parameter, `{name}`: any non-empty segment.
    Param(String),
    /// A wildcard, `{*name}`, always last: the rest of the path, slashes
    /// included, if it isn't empty.
    Wildcard(String),
}

impl Segment {
    pub(crate) fn parse(segment: &str) -> Self {
        match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(inner) => match inner.strip_prefix('*') {
                Some(name) => Segment::Wildcard(name.to_owned()),
                None => Segment::Param(inner.to_owned()),
            },
            None => Segment::Literal(segment.to_owned()),
        }
    }

    /// How specific the segment is: lower wins.
    fn rank(&self) -> u8 {
        match self {
            Segment::Literal(_) => 0,
            Segment::Param(_) => 1,
            Segment::Wildcard(_) => 2,
        }
    }

    /// Whether two segments match the same paths, whatever their names.
    fn same_shape(&self, other: &Segment) -> bool {
        match (self, other) {
            (Segment::Literal(a), Segment::Literal(b)) => a == b,
            (Segment::Param(_), Segment::Param(_))
            | (Segment::Wildcard(_), Segment::Wildcard(_)) => true,
            _ => false,
        }
    }
}

/// The path parameters of a match, by name, as they are in the path (still
/// percent-encoded).
pub(crate) type Params = Vec<(String, String)>;

#[derive(Debug, Clone)]
pub(crate) struct Route {
    segments: Vec<Segment>,
    pub(crate) method: Method,
    pub(crate) endpoint: Endpoint,
    /// The `allow` of a 405, if not `method` (Starlette's plain routes
    /// answer `GET` and `HEAD`; Starlette names them in an order that varies
    /// from run to run, as it comes from a Python set).
    pub(crate) allow: Option<&'static str>,
}

/// The result of looking up a request.
#[derive(Debug)]
pub(crate) enum Found<'a> {
    /// Path and method match; with the path parameters.
    Route(&'a Route, Params),
    /// The path matches a route with another method.
    WrongMethod(&'a Route),
    /// Nothing matches.
    Nothing,
}

/// Thing names that would clash with the server's own routes.
pub const RESERVED_THING_NAMES: [&str; 7] = [
    "things",
    "thing_descriptions",
    "action_invocations",
    "blob",
    "docs",
    "redoc",
    "directory",
];

#[derive(Default)]
pub(crate) struct Routes {
    routes: Vec<Route>,
    /// The services of custom endpoints.
    pub(crate) services: Vec<EndpointService>,
}

impl std::fmt::Debug for Routes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Routes")
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

impl Routes {
    /// Every route for the runtime's Things.
    pub(crate) fn build(runtime: &Runtime, prefix: &str) -> Result<Self, RouteError> {
        let mut routes = Routes::default();
        routes.add_get_and_head("/openapi.json", Endpoint::OpenApi);
        routes.add_get_and_head("/docs", Endpoint::SwaggerUi);
        routes.add_get_and_head("/docs/oauth2-redirect", Endpoint::OAuth2Redirect);
        routes.add_get_and_head("/redoc", Endpoint::Redoc);
        for (index, (path, _, _)) in docs::ASSETS.iter().enumerate() {
            routes.add_get_and_head(path, Endpoint::DocsAsset(index));
        }
        let invocations = format!("{prefix}/action_invocations");
        routes.add(&invocations, Method::GET, Endpoint::ListInvocations);
        routes.add(
            &format!("{invocations}/{{id}}"),
            Method::GET,
            Endpoint::GetInvocation,
        );
        routes.add(
            &format!("{invocations}/{{id}}/output"),
            Method::GET,
            Endpoint::InvocationOutput,
        );
        routes.add(
            &format!("{invocations}/{{id}}"),
            Method::DELETE,
            Endpoint::CancelInvocation,
        );
        routes.add(
            &format!("{prefix}/blob/{{id}}"),
            Method::GET,
            Endpoint::DownloadBlob,
        );
        routes.add(
            &format!("{prefix}/thing_descriptions/"),
            Method::GET,
            Endpoint::ThingDescriptions,
        );
        routes.add(
            &format!("{prefix}/things/"),
            Method::GET,
            Endpoint::ThingPaths,
        );
        // Discovery: the well-known URL is at the root, as RFC 8615
        // wants; the directory is under the prefix.
        routes.add_get_and_head("/.well-known/wot", Endpoint::WellKnown);
        routes.add_get_and_head(
            &format!("{prefix}/directory/things"),
            Endpoint::DirectoryThings,
        );
        routes.add_get_and_head(
            &format!("{prefix}/directory/things/{{id}}"),
            Endpoint::DirectoryThing,
        );

        for thing in runtime.things() {
            let name = thing.name();
            if RESERVED_THING_NAMES.contains(&name) {
                return Err(RouteError::ReservedThingName(name.to_owned()));
            }
            let base = format!("{prefix}/{name}/");
            for property in thing.properties() {
                let path = format!("{base}{}", property.name());
                let (thing, property_name) = (name.to_owned(), property.name().to_owned());
                if !property.is_read_only() {
                    routes.add(
                        &path,
                        Method::PUT,
                        Endpoint::WriteProperty {
                            thing: thing.clone(),
                            property: property_name.clone(),
                        },
                    );
                }
                routes.add(
                    &path,
                    Method::GET,
                    Endpoint::ReadProperty {
                        thing: thing.clone(),
                        property: property_name.clone(),
                    },
                );
                if property.is_resettable() {
                    routes.add(
                        &format!("{path}/reset"),
                        Method::POST,
                        Endpoint::ResetProperty {
                            thing,
                            property: property_name,
                        },
                    );
                }
            }
            for action in thing.actions() {
                let path = format!("{base}{}", action.name());
                let (thing, action) = (name.to_owned(), action.name().to_owned());
                routes.add(
                    &path,
                    Method::POST,
                    Endpoint::InvokeAction {
                        thing: thing.clone(),
                        action: action.clone(),
                    },
                );
                routes.add(
                    &path,
                    Method::GET,
                    Endpoint::ListActionInvocations { thing, action },
                );
            }
            for event in thing.events() {
                routes.add(
                    &format!("{base}{}", event.name()),
                    Method::GET,
                    Endpoint::SubscribeEvent {
                        thing: name.to_owned(),
                        event: event.name().to_owned(),
                    },
                );
            }
            for (stream, _) in thing.streams() {
                let path = format!("{base}{stream}");
                let (thing, stream) = (name.to_owned(), stream.to_owned());
                routes.add(
                    &path,
                    Method::GET,
                    Endpoint::Stream {
                        thing: thing.clone(),
                        stream: stream.clone(),
                    },
                );
                routes.add(
                    &format!("{path}/viewer"),
                    Method::GET,
                    Endpoint::StreamViewer { thing, stream },
                );
            }
            // The top-level resources, where the TD has their forms.
            let owned = || name.to_owned();
            if thing.properties().next().is_some() {
                routes.add(
                    &format!("{base}properties"),
                    Method::GET,
                    Endpoint::ReadAllProperties { thing: owned() },
                );
            }
            if thing.properties().any(|p| !p.is_read_only()) {
                routes.add(
                    &format!("{base}properties"),
                    Method::PUT,
                    Endpoint::WriteMultipleProperties { thing: owned() },
                );
            }
            if thing.actions().next().is_some() {
                routes.add(
                    &format!("{base}actions"),
                    Method::GET,
                    Endpoint::QueryAllActions { thing: owned() },
                );
            }
            if thing.events().next().is_some() {
                routes.add(
                    &format!("{base}events"),
                    Method::GET,
                    Endpoint::SubscribeAllEvents { thing: owned() },
                );
            }
            for endpoint in thing.endpoints() {
                let path = format!("{base}{}", endpoint.path());
                let conflict = || RouteError::EndpointConflict {
                    thing: name.to_owned(),
                    route: format!("{} {path}", endpoint.method()),
                };
                let method =
                    Method::from_bytes(endpoint.method().as_bytes()).map_err(|_| conflict())?;
                // A route of the same shape would match exactly the same
                // requests, so neither could be chosen over the other.
                let segments: Vec<Segment> = path.split('/').map(Segment::parse).collect();
                if routes
                    .routes
                    .iter()
                    .any(|r| r.method == method && r.same_shape(&segments))
                {
                    return Err(conflict());
                }
                let service = endpoint
                    .handler()
                    .downcast_ref::<EndpointService>()
                    .ok_or_else(|| RouteError::ForeignEndpoint {
                        thing: name.to_owned(),
                        path: endpoint.path().to_owned(),
                    })?;
                routes.services.push(service.clone());
                routes.add(&path, method, Endpoint::Custom(routes.services.len() - 1));
            }
            routes.add(
                &base,
                Method::GET,
                Endpoint::ThingDescription {
                    thing: name.to_owned(),
                },
            );
        }
        Ok(routes)
    }

    fn add(&mut self, pattern: &str, method: Method, endpoint: Endpoint) {
        let segments = pattern.split('/').map(Segment::parse).collect();
        self.routes.push(Route {
            segments,
            method,
            endpoint,
            allow: None,
        });
    }

    /// A route answering `GET` and `HEAD`.
    fn add_get_and_head(&mut self, path: &str, endpoint: Endpoint) {
        for method in [Method::GET, Method::HEAD] {
            self.add(path, method, endpoint.clone());
            if let Some(route) = self.routes.last_mut() {
                route.allow = Some("GET, HEAD");
            }
        }
    }

    /// Looks up a request by method and path. Of the routes that match, the
    /// most specific wins: compared segment by segment, a literal beats a
    /// parameter, and a parameter beats a wildcard. Between equals, the
    /// first added wins.
    pub(crate) fn find(&self, method: &Method, path: &str) -> Found<'_> {
        let mut best: Option<(&Route, Params)> = None;
        let mut wrong_method: Option<&Route> = None;
        for route in &self.routes {
            let Some(params) = route.matches(path) else {
                continue;
            };
            if route.method != *method {
                if wrong_method.is_none_or(|w| route.more_specific_than(w)) {
                    wrong_method = Some(route);
                }
            } else if best
                .as_ref()
                .is_none_or(|(b, _)| route.more_specific_than(b))
            {
                best = Some((route, params));
            }
        }
        match (best, wrong_method) {
            (Some((route, params)), _) => Found::Route(route, params),
            (None, Some(route)) => Found::WrongMethod(route),
            (None, None) => Found::Nothing,
        }
    }

    /// Whether any route has this path, whatever its method.
    pub(crate) fn has_path(&self, path: &str) -> bool {
        self.routes.iter().any(|r| r.matches(path).is_some())
    }
}

impl Route {
    /// The path parameters, if the path matches.
    fn matches(&self, path: &str) -> Option<Params> {
        let parts: Vec<&str> = path.split('/').collect();
        let wildcard = match self.segments.last() {
            Some(Segment::Wildcard(name)) => Some(name),
            _ => None,
        };
        let fixed = self.segments.len() - usize::from(wildcard.is_some());
        let enough = match wildcard {
            Some(_) => parts.len() > fixed,
            None => parts.len() == fixed,
        };
        if !enough {
            return None;
        }
        let mut params = Vec::new();
        for (segment, part) in self.segments[..fixed].iter().zip(&parts) {
            match segment {
                Segment::Literal(literal) if literal == *part => {}
                Segment::Param(name) if !part.is_empty() => {
                    params.push((name.clone(), (*part).to_owned()));
                }
                _ => return None,
            }
        }
        if let Some(name) = wildcard {
            let rest = parts[fixed..].join("/");
            if rest.is_empty() {
                return None;
            }
            params.push((name.clone(), rest));
        }
        Some(params)
    }

    /// Whether this route wins over `other` when both match a path.
    fn more_specific_than(&self, other: &Route) -> bool {
        self.segments
            .iter()
            .map(Segment::rank)
            .lt(other.segments.iter().map(Segment::rank))
    }

    /// Whether this route's pattern matches exactly the paths `segments`
    /// does.
    fn same_shape(&self, segments: &[Segment]) -> bool {
        self.segments.len() == segments.len()
            && self
                .segments
                .iter()
                .zip(segments)
                .all(|(a, b)| a.same_shape(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Routes {
        let mut routes = Routes::default();
        routes.add("/t/p", Method::PUT, Endpoint::ListInvocations);
        routes.add("/t/p", Method::GET, Endpoint::ThingPaths);
        routes.add("/a/{id}", Method::GET, Endpoint::GetInvocation);
        routes.add("/a/{id}/output", Method::GET, Endpoint::InvocationOutput);
        routes.add("/t/", Method::GET, Endpoint::ThingDescriptions);
        routes
    }

    fn param(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn full_matches_are_found_with_their_parameters() {
        let routes = table();
        assert!(
            matches!(routes.find(&Method::GET, "/t/p"), Found::Route(r, p) if r.endpoint == Endpoint::ThingPaths && p.is_empty())
        );
        assert!(
            matches!(routes.find(&Method::GET, "/a/xyz"), Found::Route(_, p) if p == [param("id", "xyz")])
        );
        assert!(
            matches!(routes.find(&Method::GET, "/a/xyz/output"), Found::Route(r, _) if r.endpoint == Endpoint::InvocationOutput)
        );
    }

    #[test]
    fn a_wrong_method_reports_the_first_route() {
        let routes = table();
        assert!(
            matches!(routes.find(&Method::DELETE, "/t/p"), Found::WrongMethod(r) if r.method == Method::PUT)
        );
        assert!(
            matches!(routes.find(&Method::HEAD, "/t/"), Found::WrongMethod(r) if r.method == Method::GET)
        );
    }

    #[test]
    fn trailing_slashes_matter() {
        let routes = table();
        assert!(matches!(routes.find(&Method::GET, "/t"), Found::Nothing));
        assert!(routes.has_path("/t/"));
        assert!(
            matches!(routes.find(&Method::GET, "/a/"), Found::Nothing),
            "an empty parameter doesn't match"
        );
    }

    #[test]
    fn literals_beat_parameters_and_parameters_beat_wildcards() {
        let mut routes = Routes::default();
        // Added least specific first: the order mustn't decide.
        routes.add("/x/{*rest}", Method::GET, Endpoint::DownloadBlob);
        routes.add("/x/{a}", Method::GET, Endpoint::OpenApi);
        routes.add("/x/{a}/c", Method::GET, Endpoint::ListInvocations);
        routes.add("/x/b/{c}", Method::GET, Endpoint::ThingPaths);
        routes.add("/x/b/c", Method::GET, Endpoint::GetInvocation);
        let found = |path| match routes.find(&Method::GET, path) {
            Found::Route(route, params) => Some((route.endpoint.clone(), params)),
            _ => None,
        };
        assert_eq!(found("/x/b/c"), Some((Endpoint::GetInvocation, vec![])));
        assert_eq!(
            found("/x/b/z"),
            Some((Endpoint::ThingPaths, vec![param("c", "z")]))
        );
        assert_eq!(
            found("/x/q/c"),
            Some((Endpoint::ListInvocations, vec![param("a", "q")]))
        );
        assert_eq!(
            found("/x/q"),
            Some((Endpoint::OpenApi, vec![param("a", "q")]))
        );
        assert_eq!(
            found("/x/q/r/s"),
            Some((Endpoint::DownloadBlob, vec![param("rest", "q/r/s")]))
        );
        assert_eq!(found("/x/"), None, "an empty rest doesn't match");
    }

    #[test]
    fn routes_of_the_same_shape_are_recognised_whatever_their_names() {
        let mut routes = Routes::default();
        routes.add("/x/{a}/c", Method::GET, Endpoint::ListInvocations);
        let shape =
            |pattern: &str| -> Vec<Segment> { pattern.split('/').map(Segment::parse).collect() };
        assert!(routes.routes[0].same_shape(&shape("/x/{other}/c")));
        assert!(!routes.routes[0].same_shape(&shape("/x/b/c")));
        assert!(!routes.routes[0].same_shape(&shape("/x/{*rest}")));
    }
}
