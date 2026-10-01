//! The route table.

use http::Method;
use teta_wot_core::Runtime;

use crate::RouteError;

/// What a route does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Endpoint {
    ThingDescription { thing: String },
    ReadProperty { thing: String, property: String },
    WriteProperty { thing: String, property: String },
    ResetProperty { thing: String, property: String },
    InvokeAction { thing: String, action: String },
    ListActionInvocations { thing: String, action: String },
    ListInvocations,
    GetInvocation,
    CancelInvocation,
    InvocationOutput,
    ThingPaths,
    ThingDescriptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    /// A path parameter (the invocation ID): any non-empty segment.
    Param,
}

#[derive(Debug, Clone)]
pub(crate) struct Route {
    segments: Vec<Segment>,
    pub(crate) method: Method,
    pub(crate) endpoint: Endpoint,
}

/// The result of looking up a request.
#[derive(Debug)]
pub(crate) enum Found<'a> {
    /// Path and method match; with the path parameter, if any.
    Route(&'a Route, Option<String>),
    /// The path matches a route with another method.
    WrongMethod(&'a Route),
    /// Nothing matches.
    Nothing,
}

/// Thing names that would clash with the server's own routes.
pub const RESERVED_THING_NAMES: [&str; 6] = [
    "things",
    "thing_descriptions",
    "action_invocations",
    "blob",
    "docs",
    "redoc",
];

#[derive(Debug, Default)]
pub(crate) struct Routes {
    routes: Vec<Route>,
}

impl Routes {
    /// Every route for the runtime's Things, in registration order.
    pub(crate) fn build(runtime: &Runtime, prefix: &str) -> Result<Self, RouteError> {
        let mut routes = Routes::default();
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
            &format!("{prefix}/thing_descriptions/"),
            Method::GET,
            Endpoint::ThingDescriptions,
        );
        routes.add(
            &format!("{prefix}/things/"),
            Method::GET,
            Endpoint::ThingPaths,
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
        let segments = pattern
            .split('/')
            .map(|s| {
                if s == "{id}" {
                    Segment::Param
                } else {
                    Segment::Literal(s.to_owned())
                }
            })
            .collect();
        self.routes.push(Route {
            segments,
            method,
            endpoint,
        });
    }

    /// Looks up a request by method and path.
    pub(crate) fn find(&self, method: &Method, path: &str) -> Found<'_> {
        let mut wrong_method = None;
        for route in &self.routes {
            if let Some(param) = route.matches(path) {
                if route.method == *method {
                    return Found::Route(route, param);
                }
                wrong_method.get_or_insert(route);
            }
        }
        wrong_method.map_or(Found::Nothing, Found::WrongMethod)
    }

    /// Whether any route has this path, whatever its method.
    pub(crate) fn has_path(&self, path: &str) -> bool {
        self.routes.iter().any(|r| r.matches(path).is_some())
    }
}

impl Route {
    /// `Some(param)` if the path matches (`param` is the path parameter, if any).
    fn matches(&self, path: &str) -> Option<Option<String>> {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != self.segments.len() {
            return None;
        }
        let mut param = None;
        for (segment, part) in self.segments.iter().zip(parts) {
            match segment {
                Segment::Literal(literal) if literal == part => {}
                Segment::Param if !part.is_empty() => param = Some(part.to_owned()),
                _ => return None,
            }
        }
        Some(param)
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

    #[test]
    fn the_first_full_match_wins() {
        let routes = table();
        assert!(
            matches!(routes.find(&Method::GET, "/t/p"), Found::Route(r, None) if r.endpoint == Endpoint::ThingPaths)
        );
        assert!(
            matches!(routes.find(&Method::GET, "/a/xyz"), Found::Route(_, Some(id)) if id == "xyz")
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
}
