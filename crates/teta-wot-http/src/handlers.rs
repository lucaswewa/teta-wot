//! Request handlers.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use http::{HeaderValue, Method, StatusCode};
use serde_json::{Map, Value, json};
use teta_wot_core::invocation::CancelError;
use teta_wot_core::{LocItem, PropertyError, Runtime, TdOptions, ValidationIssue};
use tower::ServiceExt;
use uuid::Uuid;

use crate::render::{self, Urls, created, detail, problem, redirect, unprocessable};
use crate::routes::{Endpoint, Found, Routes};
use crate::{HttpOptions, td_id};

/// The largest request body read (64 MiB).
const BODY_LIMIT: usize = 64 * 1024 * 1024;

pub(crate) struct App {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) routes: Routes,
    pub(crate) options: HttpOptions,
}

/// The one handler: looks the request up in the route table and answers.
pub(crate) async fn dispatch(State(app): State<Arc<App>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let head = parts.method == Method::HEAD;
    let urls = Urls::from_request(&parts, &app.options.api_prefix);
    let path = parts.uri.path().to_owned();
    let mut response = match app.routes.find(&parts.method, &path) {
        Found::Route(route, param) => match route.endpoint {
            // A custom endpoint gets the whole request, for its extractors.
            Endpoint::Custom(index) => {
                let service = app.routes.services[index].clone();
                match service.oneshot(Request::from_parts(parts, body)).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            }
            _ => app.endpoint(&route.endpoint, param, &urls, body).await,
        },
        Found::WrongMethod(route) => {
            let mut response = detail(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
            if let Ok(allow) = HeaderValue::from_str(route.method.as_str()) {
                response.headers_mut().insert(http::header::ALLOW, allow);
            }
            response
        }
        Found::Nothing => {
            let toggled = match path.strip_suffix('/') {
                Some(_) => path.trim_end_matches('/').to_owned(),
                None => format!("{path}/"),
            };
            if path != "/" && app.routes.has_path(&toggled) {
                let query = parts
                    .uri
                    .query()
                    .map(|q| format!("?{q}"))
                    .unwrap_or_default();
                redirect(&urls.absolute(&format!("{toggled}{query}")))
            } else {
                detail(StatusCode::NOT_FOUND, "Not Found")
            }
        }
    };
    if head {
        *response.body_mut() = Body::empty();
    }
    response
}

impl App {
    async fn endpoint(
        &self,
        endpoint: &Endpoint,
        param: Option<String>,
        urls: &Urls,
        body: Body,
    ) -> Response {
        let invocations = self.runtime.invocations();
        match endpoint {
            // Answered by `dispatch`, which passes on the whole request.
            Endpoint::Custom(_) => {
                detail(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
            Endpoint::ThingDescription { thing } => match self.td(thing, urls) {
                Ok(td) => render::json(StatusCode::OK, &td),
                Err(error) => detail(StatusCode::INTERNAL_SERVER_ERROR, &error),
            },
            Endpoint::ThingDescriptions => {
                let mut out = Map::new();
                for thing in self.runtime.things() {
                    match self.td(thing.name(), urls) {
                        Ok(td) => out.insert(thing.name().to_owned(), td),
                        Err(error) => return detail(StatusCode::INTERNAL_SERVER_ERROR, &error),
                    };
                }
                render::json(StatusCode::OK, &Value::Object(out))
            }
            Endpoint::ThingPaths => {
                let paths: Map<String, Value> = self
                    .runtime
                    .things()
                    .map(|t| {
                        (
                            t.name().to_owned(),
                            json!(urls.absolute(&urls.thing_path(t.name()))),
                        )
                    })
                    .collect();
                render::json(StatusCode::OK, &Value::Object(paths))
            }
            Endpoint::ReadProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return detail(StatusCode::NOT_FOUND, "Not Found");
                };
                match entry.read().await {
                    Ok(value) => render::json(StatusCode::OK, &value),
                    Err(error) => property_error(thing, error),
                }
            }
            Endpoint::WriteProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return detail(StatusCode::NOT_FOUND, "Not Found");
                };
                let value = match read_json(body).await {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                match entry.write(value).await {
                    Ok(()) => render::json(StatusCode::CREATED, &Value::Null),
                    Err(error) => property_error(thing, error),
                }
            }
            Endpoint::ResetProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return detail(StatusCode::NOT_FOUND, "Not Found");
                };
                match entry.reset().await {
                    Ok(()) => render::json(StatusCode::OK, &Value::Null),
                    Err(error) => property_error(thing, error),
                }
            }
            Endpoint::InvokeAction { thing, action } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.action(action)) else {
                    return detail(StatusCode::NOT_FOUND, "Not Found");
                };
                let input = match read_json(body).await {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                match entry.invoke(input) {
                    Ok(invocation) => created(&invocation.record(), urls),
                    Err(error) => unprocessable(&error.issues),
                }
            }
            Endpoint::ListActionInvocations { thing, action } => {
                let list: Vec<Value> = invocations
                    .list_for(thing, action)
                    .iter()
                    .map(|i| render::invocation(&i.record(), urls, false))
                    .collect();
                render::json(StatusCode::OK, &Value::Array(list))
            }
            Endpoint::ListInvocations => {
                let list: Vec<Value> = invocations
                    .list()
                    .iter()
                    .map(|i| render::invocation(&i.record(), urls, false))
                    .collect();
                render::json(StatusCode::OK, &Value::Array(list))
            }
            Endpoint::GetInvocation => {
                let id = match parse_id(param.as_deref()) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                match invocations.get(id) {
                    Some(invocation) => render::json(
                        StatusCode::OK,
                        &render::invocation(&invocation.record(), urls, true),
                    ),
                    None => detail(
                        StatusCode::NOT_FOUND,
                        &CancelError::NotFound(id).to_string(),
                    ),
                }
            }
            Endpoint::CancelInvocation => {
                let id = match parse_id(param.as_deref()) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                match invocations.cancel(id) {
                    Ok(()) => render::json(StatusCode::OK, &Value::Null),
                    Err(error @ CancelError::NotFound(_)) => {
                        detail(StatusCode::NOT_FOUND, &error.to_string())
                    }
                    Err(error @ CancelError::NotCancellable(_)) => {
                        detail(StatusCode::SERVICE_UNAVAILABLE, &error.to_string())
                    }
                }
            }
            Endpoint::InvocationOutput => {
                let id = match parse_id(param.as_deref()) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(invocation) = invocations.get(id) else {
                    return detail(
                        StatusCode::NOT_FOUND,
                        &CancelError::NotFound(id).to_string(),
                    );
                };
                // Reference bug B3 fixed: falsy outputs such as 0 are returned;
                // only a missing or null output is "no result".
                match invocation.output() {
                    Some(output) if !output.is_null() => render::json(StatusCode::OK, &output),
                    _ => detail(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "No result is available for this invocation",
                    ),
                }
            }
        }
    }

    /// A Thing's TD as served: with the request's base URL and a stable `id`.
    fn td(&self, thing: &str, urls: &Urls) -> Result<Value, String> {
        let handle = self
            .runtime
            .thing(thing)
            .ok_or_else(|| format!("no Thing named {thing}"))?;
        let options = TdOptions {
            path: urls.thing_path(thing),
            base: Some(urls.base()),
            id: Some(td_id(&self.options.server_id, thing)),
        };
        let td = handle
            .thing_description(&options)
            .map_err(|e| e.to_string())?;
        serde_json::to_value(td).map_err(|e| e.to_string())
    }
}

/// A property failure: 422 for invalid values, otherwise problem details
/// (409 for a busy global lock, 500 for the rest).
fn property_error(thing: &str, error: PropertyError) -> Response {
    match error {
        PropertyError::Invalid(invalid) => unprocessable(&invalid.issues),
        other => {
            tracing::error!(target: "wot::things", thing = %thing, "{other}");
            problem(&other.problem())
        }
    }
}

/// Reads a JSON body. An empty body is `null`; malformed JSON is `json_invalid` error,
/// located at the character where parsing failed.
#[allow(clippy::result_large_err)] // the error is the response to send
async fn read_json(body: Body) -> Result<Value, Response> {
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => return Err(detail(StatusCode::PAYLOAD_TOO_LARGE, &error.to_string())),
    };
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        let text = String::from_utf8_lossy(&bytes);
        let position = char_offset(&text, error.line(), error.column());
        let message = error.to_string();
        let message = message
            .split(" at line ")
            .next()
            .unwrap_or(&message)
            .to_owned();
        let mut ctx = Map::new();
        ctx.insert("error".into(), json!(message));
        unprocessable(&[ValidationIssue {
            kind: "json_invalid".into(),
            loc: vec![LocItem::from("body"), LocItem::Index(position)],
            msg: "JSON decode error".into(),
            input: json!({}),
            ctx: Some(ctx),
        }])
    })
}

/// The character offset of a 1-based line and column.
fn char_offset(text: &str, line: usize, column: usize) -> usize {
    let before: usize = text
        .split('\n')
        .take(line.saturating_sub(1))
        .map(|l| l.chars().count() + 1)
        .sum();
    before + column.saturating_sub(1)
}

/// Parses the invocation ID in the path, or answers 422.
#[allow(clippy::result_large_err)]
fn parse_id(raw: Option<&str>) -> Result<Uuid, Response> {
    let raw = raw.unwrap_or_default();
    Uuid::parse_str(raw).map_err(|error| {
        let mut ctx = Map::new();
        ctx.insert("error".into(), json!(error.to_string()));
        unprocessable(&[ValidationIssue {
            kind: "uuid_parsing".into(),
            loc: vec![LocItem::from("path"), LocItem::from("id")],
            msg: format!("Input should be a valid UUID, {error}"),
            input: json!(raw),
            ctx: Some(ctx),
        }])
    })
}
