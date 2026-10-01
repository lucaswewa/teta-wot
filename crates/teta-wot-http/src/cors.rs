//! CORS: every origin, method and header allowed, with credentials.
//!
//! - every response carries `vary: Origin`;
//! - a request with an `Origin` gets it mirrored in
//!   `access-control-allow-origin`, with `access-control-allow-credentials: true`;
//! - a preflight (`OPTIONS` with `Origin` and
//!   `Access-Control-Request-Method`) is answered directly, on any path, with
//!   200 `OK` (`text/plain; charset=utf-8`), the mirrored origin, every
//!   method, the requested headers, a max age of 600 s, and Starlette's
//!   `vary` list.

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS,
    ACCESS_CONTROL_REQUEST_METHOD, CONTENT_TYPE, ORIGIN, VARY,
};
use http::{HeaderValue, Method, StatusCode};

const ALL_METHODS: &str = "DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY";
const PREFLIGHT_VARY: &str = "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network";

pub(crate) async fn cors(request: Request, next: Next) -> Response {
    let origin = request.headers().get(ORIGIN).cloned();
    if let Some(origin) = &origin
        && request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(ACCESS_CONTROL_REQUEST_METHOD)
    {
        return preflight(
            origin,
            request.headers().get(ACCESS_CONTROL_REQUEST_HEADERS),
        );
    }

    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    match headers.get(VARY).and_then(|v| v.to_str().ok()) {
        Some(existing) => {
            if let Ok(vary) = HeaderValue::from_str(&format!("{existing}, Origin")) {
                headers.insert(VARY, vary);
            }
        }
        None => {
            headers.insert(VARY, HeaderValue::from_static("Origin"));
        }
    }
    if let Some(origin) = origin {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }
    response
}

fn preflight(origin: &HeaderValue, requested_headers: Option<&HeaderValue>) -> Response {
    let mut response = Response::new(Body::from("OK"));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALL_METHODS),
    );
    if let Some(requested) = requested_headers {
        headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, requested.clone());
    }
    headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    headers.insert(VARY, HeaderValue::from_static(PREFLIGHT_VARY));
    response
}
