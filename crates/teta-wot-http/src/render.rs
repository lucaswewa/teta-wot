//! Rendering responses: absolute URLs, invocations, errors.

use axum::body::Body;
use axum::response::Response;
use chrono::{DateTime, Utc};
use http::header::{CONTENT_TYPE, LOCATION};
use http::request::Parts;
use http::{HeaderValue, StatusCode};
use serde_json::{Map, Value, json};
use teta_wot_core::{InvocationRecord, ProblemDetails, ValidationIssue};

/// The URLs of the request being answered: absolute URLs in responses are
/// built from its scheme and host at the time they are written, never
/// stored.
#[derive(Debug, Clone)]
pub(crate) struct Urls {
    /// `scheme://host[:port]`, without a trailing slash.
    pub(crate) origin: String,
    /// The API prefix, such as `/api/v1`, or empty.
    pub(crate) prefix: String,
}

impl Urls {
    pub(crate) fn from_request(parts: &Parts, prefix: &str) -> Self {
        let scheme = parts.uri.scheme_str().unwrap_or("http");
        let host = parts
            .headers
            .get(http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned)
            .or_else(|| parts.uri.authority().map(|a| a.to_string()))
            .unwrap_or_else(|| "localhost".to_owned());
        Self {
            origin: format!("{scheme}://{host}"),
            prefix: prefix.to_owned(),
        }
    }

    /// The base URL, as the TD's `base`: `scheme://host/`.
    pub(crate) fn base(&self) -> String {
        format!("{}/", self.origin)
    }

    /// The absolute URL of a path on this server (the path includes the prefix).
    pub(crate) fn absolute(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    pub(crate) fn thing_path(&self, thing: &str) -> String {
        format!("{}/{thing}/", self.prefix)
    }

    /// The absolute URL of a Thing's WebSocket: `ws://` (or `wss://`) and
    /// the request's host.
    pub(crate) fn websocket(&self, thing: &str) -> String {
        let origin = match self.origin.split_once("://") {
            Some(("https", host)) => format!("wss://{host}"),
            Some((_, host)) => format!("ws://{host}"),
            None => self.origin.clone(),
        };
        format!("{origin}{}ws", self.thing_path(thing))
    }

    pub(crate) fn invocation_href(&self, id: &uuid::Uuid) -> String {
        self.absolute(&format!("{}/action_invocations/{id}", self.prefix))
    }
}

/// A naive local time with microseconds, left out when zero.
pub(crate) fn naive_local(time: &DateTime<Utc>) -> String {
    let local = time.with_timezone(&chrono::Local).naive_local();
    if local.and_utc().timestamp_subsec_micros() == 0 {
        local.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        local.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    }
}

/// An invocation in JSON: the summary fields, and with `full`
/// also `input`, `output`, `log` and `error`.
pub(crate) fn invocation(record: &InvocationRecord, urls: &Urls, full: bool) -> Value {
    let href = urls.invocation_href(&record.id);
    let link =
        |href: String, rel: &str| json!({"href": href, "type": null, "rel": rel, "anchor": null});
    let mut out = Map::new();
    out.insert("status".into(), json!(record.status.as_str()));
    out.insert("id".into(), json!(record.id));
    out.insert(
        "action".into(),
        json!(format!(
            "{}{}",
            urls.thing_path(&record.thing),
            record.action
        )),
    );
    out.insert("href".into(), json!(href));
    out.insert(
        "timeStarted".into(),
        json!(record.time_started.as_ref().map(naive_local)),
    );
    out.insert(
        "timeRequested".into(),
        json!(naive_local(&record.time_requested)),
    );
    out.insert(
        "timeCompleted".into(),
        json!(record.time_completed.as_ref().map(naive_local)),
    );
    out.insert(
        "links".into(),
        json!([
            link(href.clone(), "self"),
            link(format!("{href}/output"), "output")
        ]),
    );
    if full {
        let mut input = record.input.clone();
        let mut output = record.output.clone().unwrap_or(Value::Null);
        crate::output::resolve_blobs(&mut input, urls);
        crate::output::resolve_blobs(&mut output, urls);
        out.insert("input".into(), input);
        out.insert("output".into(), output);
        out.insert(
            "log".into(),
            serde_json::to_value(&record.log).unwrap_or_default(),
        );
        out.insert(
            "error".into(),
            serde_json::to_value(&record.error).unwrap_or_default(),
        );
    }
    Value::Object(out)
}

/// A JSON response.
pub(crate) fn json(status: StatusCode, value: &Value) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

/// `{"detail": message}`.
pub(crate) fn detail(status: StatusCode, message: &str) -> Response {
    json(status, &json!({ "detail": message }))
}

/// A 422 with pydantic-style errors.
pub(crate) fn unprocessable(issues: &[ValidationIssue]) -> Response {
    json(
        StatusCode::UNPROCESSABLE_ENTITY,
        &json!({ "detail": issues }),
    )
}

/// A problem details response, with the problem's status (500 if none).
pub(crate) fn problem(problem: &ProblemDetails) -> Response {
    let status = problem
        .status
        .and_then(|s| StatusCode::from_u16(s).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json(status, &serde_json::to_value(problem).unwrap_or_default())
}

/// A 201 for a new invocation, with its URL in `Location`.
pub(crate) fn created(record: &InvocationRecord, urls: &Urls) -> Response {
    let mut response = json(StatusCode::CREATED, &invocation(record, urls, true));
    if let Ok(location) = HeaderValue::from_str(&urls.invocation_href(&record.id)) {
        response.headers_mut().insert(LOCATION, location);
    }
    response
}

/// A 307 redirect to an absolute URL.
pub(crate) fn redirect(location: &str) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    if let Ok(location) = HeaderValue::from_str(location) {
        response.headers_mut().insert(LOCATION, location);
    }
    response
}
