//! Request handlers.

use std::sync::Arc;

use axum::body::{Body, HttpBody};
use axum::extract::{Request, State};
use axum::response::Response;
use http::{HeaderValue, Method, StatusCode};
use serde_json::{Map, Value, json};
use teta_wot_core::blob::BlobData;
use teta_wot_core::invocation::{CancelError, InvocationStatus};
use teta_wot_core::{
    InvocationRecord, LocItem, MessageKind, ProblemDetails, PropertyError, Runtime, TdOptions,
    ValidationIssue,
};
use tower::ServiceExt;
use uuid::Uuid;

use crate::problem::HttpError;
use crate::render::{self, Urls, created, redirect};
use crate::routes::{Endpoint, Found, Route, Routes};
use crate::{HTTP_BASIC_PROFILE, HTTP_SSE_PROFILE, HttpOptions, WireProfile, td_id};
use crate::{discovery, docs, observe, output, security};

/// The largest request body read (64 MiB).
const BODY_LIMIT: usize = 64 * 1024 * 1024;

pub(crate) struct App {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) routes: Routes,
    pub(crate) options: HttpOptions,
    /// The OpenAPI document, serialised once: it doesn't change.
    pub(crate) openapi: bytes::Bytes,
}

/// The one handler: looks the request up in the route table and answers.
pub(crate) async fn dispatch(State(app): State<Arc<App>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    // WebSocket upgrades aren't in the route table: like Starlette's
    // WebSocket routes, `/{thing}/ws` doesn't answer plain HTTP.
    if let Some(thing) = observe::websocket_thing(&app.runtime, &app.options.api_prefix, &parts) {
        drain(body).await;
        if let Some(denied) = app.deny(&parts.headers) {
            return denied;
        }
        let urls = Urls::from_request(&parts, &app.options.api_prefix);
        return observe::upgrade(Arc::clone(app.runtime.broker()), thing, urls, parts).await;
    }
    let head = parts.method == Method::HEAD;
    let urls = Urls::from_request(&parts, &app.options.api_prefix);
    let path = parts.uri.path().to_owned();
    let mut response = match app.routes.find(&parts.method, &path) {
        Found::Route(route, param) => {
            match app
                .deny(&parts.headers)
                .filter(|_| !route.endpoint.is_public())
            {
                Some(denied) => {
                    drain(body).await;
                    denied
                }
                None => app.route(route, param, parts, body, urls).await,
            }
        }
        Found::WrongMethod(route) => {
            drain(body).await;
            let profile = if route.endpoint.is_discovery() {
                WireProfile::Wot
            } else {
                app.options.profile
            };
            let mut response = HttpError::Status(
                StatusCode::METHOD_NOT_ALLOWED,
                "Method Not Allowed".to_owned(),
            )
            .response(profile);
            if let Ok(allow) = HeaderValue::from_str(route.allow.unwrap_or(route.method.as_str())) {
                response.headers_mut().insert(http::header::ALLOW, allow);
            }
            response
        }
        Found::Nothing => {
            drain(body).await;
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
                app.error(HttpError::not_found())
            }
        }
    };
    if head {
        // The headers are those of a `GET`, including its length.
        if let Some(length) = response.body().size_hint().exact()
            && !response
                .headers()
                .contains_key(http::header::CONTENT_LENGTH)
        {
            response
                .headers_mut()
                .insert(http::header::CONTENT_LENGTH, HeaderValue::from(length));
        }
        *response.body_mut() = Body::empty();
    }
    response
}

/// Reads and discards a request body that the answer doesn't need. A
/// connection closed with unread data in it is reset rather than closed,
/// and the client may then lose the answer (seen on Windows, answering a
/// `PUT` with 405).
async fn drain(body: Body) {
    let _ = axum::body::to_bytes(body, BODY_LIMIT).await;
}

impl App {
    fn wot(&self) -> bool {
        self.options.profile == WireProfile::Wot
    }

    /// An error response in the server's profile.
    fn error(&self, error: HttpError<'_>) -> Response {
        error.response(self.options.profile)
    }

    /// The 401 to send, if credentials are required and missing.
    fn deny(&self, headers: &http::HeaderMap) -> Option<Response> {
        let security = self.options.security.as_ref()?;
        (!security::authorized(security, headers))
            .then(|| security::unauthorized(security, self.options.profile))
    }

    /// Answers a request that matched `route`.
    async fn route(
        &self,
        route: &Route,
        param: Option<String>,
        parts: http::request::Parts,
        body: Body,
        urls: Urls,
    ) -> Response {
        let broker = self.runtime.broker();
        match &route.endpoint {
            // A custom endpoint gets the whole request, for its extractors.
            Endpoint::Custom(index) => {
                let service = self.routes.services[*index].clone();
                match service.oneshot(Request::from_parts(parts, body)).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            }
            Endpoint::ReadProperty { thing, property }
                if observe::wants_event_stream(&parts.headers)
                    && self
                        .runtime
                        .thing(thing)
                        .and_then(|t| t.property(property))
                        .is_some_and(|p| p.is_observable()) =>
            {
                drain(body).await;
                observe::event_stream(broker, thing, property, urls, self.wot())
            }
            Endpoint::SubscribeEvent { thing, event } => {
                drain(body).await;
                observe::event_stream(broker, thing, event, urls, self.wot())
            }
            Endpoint::ReadAllProperties { thing }
                if observe::wants_event_stream(&parts.headers) =>
            {
                drain(body).await;
                match self.runtime.thing(thing) {
                    Some(handle) => {
                        observe::all_stream(broker, handle, MessageKind::Property, urls)
                    }
                    None => self.error(HttpError::not_found()),
                }
            }
            Endpoint::SubscribeAllEvents { thing } => {
                drain(body).await;
                match self.runtime.thing(thing) {
                    Some(handle) => observe::all_stream(broker, handle, MessageKind::Event, urls),
                    None => self.error(HttpError::not_found()),
                }
            }
            Endpoint::Stream { thing, stream } => {
                drain(body).await;
                match self.runtime.thing(thing).and_then(|t| t.stream(stream)) {
                    Some(stream) => output::mjpeg_response(stream, Arc::clone(broker)),
                    None => self.error(HttpError::not_found()),
                }
            }
            Endpoint::StreamViewer { thing, stream } => {
                drain(body).await;
                output::viewer_page(&format!("{}{stream}", urls.thing_path(thing)))
            }
            Endpoint::DirectoryThings => {
                drain(body).await;
                match self.all_tds(&urls) {
                    Ok(tds) => discovery::listing(tds, &parts.uri, &urls),
                    Err(error) => server_error(&error),
                }
            }
            Endpoint::WriteProperty { .. }
            | Endpoint::InvokeAction { .. }
            | Endpoint::WriteMultipleProperties { .. } => {
                self.endpoint(&route.endpoint, param, &urls, body).await
            }
            _ => {
                drain(body).await;
                self.endpoint(&route.endpoint, param, &urls, Body::empty())
                    .await
            }
        }
    }

    async fn endpoint(
        &self,
        endpoint: &Endpoint,
        param: Option<String>,
        urls: &Urls,
        body: Body,
    ) -> Response {
        let invocations = self.runtime.invocations();
        match endpoint {
            // Answered by `route`, which needs the whole request.
            Endpoint::Custom(_)
            | Endpoint::SubscribeEvent { .. }
            | Endpoint::SubscribeAllEvents { .. }
            | Endpoint::Stream { .. }
            | Endpoint::StreamViewer { .. }
            | Endpoint::DirectoryThings => server_error("Internal Server Error"),
            Endpoint::OpenApi => {
                let mut response = Response::new(Body::from(self.openapi.clone()));
                response.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                response
            }
            Endpoint::SwaggerUi => docs::swagger_ui(&self.options.api_title),
            Endpoint::OAuth2Redirect => docs::oauth2_redirect(),
            Endpoint::Redoc => docs::redoc(&self.options.api_title),
            Endpoint::DocsAsset(index) => docs::asset(*index),
            Endpoint::ThingDescription { thing } => match self.td(thing, urls) {
                Ok(td) => render::json(StatusCode::OK, &td),
                Err(error) => {
                    self.error(HttpError::Status(StatusCode::INTERNAL_SERVER_ERROR, error))
                }
            },
            Endpoint::ThingDescriptions => {
                let mut out = Map::new();
                for thing in self.runtime.things() {
                    match self.td(thing.name(), urls) {
                        Ok(td) => out.insert(thing.name().to_owned(), td),
                        Err(error) => {
                            return self.error(HttpError::Status(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                error,
                            ));
                        }
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
            Endpoint::WellKnown => discovery::json_as(
                "application/td+json",
                &discovery::directory_td(&self.options, urls),
            ),
            Endpoint::DirectoryThing => match self.all_tds(urls) {
                Ok(tds) => discovery::retrieve(tds, param.as_deref().unwrap_or_default()),
                Err(error) => server_error(&error),
            },
            Endpoint::ReadProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return self.error(HttpError::not_found());
                };
                match entry.read().await {
                    Ok(mut value) => {
                        output::resolve_blobs(&mut value, urls);
                        render::json(StatusCode::OK, &value)
                    }
                    Err(error) => self.property_error(thing, error),
                }
            }
            Endpoint::WriteProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return self.error(HttpError::not_found());
                };
                let value = match self.read_json(body).await {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                match entry.write(value).await {
                    Ok(()) => self.written(StatusCode::CREATED),
                    Err(error) => self.property_error(thing, error),
                }
            }
            Endpoint::ResetProperty { thing, property } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.property(property))
                else {
                    return self.error(HttpError::not_found());
                };
                match entry.reset().await {
                    Ok(()) => self.written(StatusCode::OK),
                    Err(error) => self.property_error(thing, error),
                }
            }
            Endpoint::ReadAllProperties { thing } => {
                let Some(handle) = self.runtime.thing(thing) else {
                    return self.error(HttpError::not_found());
                };
                let mut properties: Vec<_> = handle.properties().collect();
                properties.sort_by(|a, b| a.name().cmp(b.name()));
                let mut values = Map::new();
                for property in properties {
                    match property.read().await {
                        Ok(mut value) => {
                            output::resolve_blobs(&mut value, urls);
                            values.insert(property.name().to_owned(), value);
                        }
                        Err(error) => return self.property_error(thing, error),
                    }
                }
                render::json(StatusCode::OK, &Value::Object(values))
            }
            Endpoint::WriteMultipleProperties { thing } => {
                let value = match self.read_json(body).await {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                self.write_multiple(thing, value).await
            }
            Endpoint::InvokeAction { thing, action } => {
                let Some(entry) = self.runtime.thing(thing).and_then(|t| t.action(action)) else {
                    return self.error(HttpError::not_found());
                };
                let input = match self.read_json(body).await {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                let invocation = match entry.invoke(input) {
                    Ok(invocation) => invocation,
                    Err(error) => return self.error(HttpError::Invalid(&error.issues)),
                };
                if self.wot() && entry.is_synchronous() {
                    // A synchronous action's caller waits for its output. If
                    // the caller goes away, the action still finishes.
                    invocation.wait().await;
                    let record = invocation.record();
                    return match record.status {
                        InvocationStatus::Completed if !entry.has_output() => render::no_content(),
                        InvocationStatus::Completed => {
                            let mut output = record.output.unwrap_or(Value::Null);
                            output::resolve_blobs(&mut output, urls);
                            render::json(StatusCode::OK, &output)
                        }
                        _ => self.error(HttpError::Problem(
                            record.error.unwrap_or_else(cancelled_problem),
                        )),
                    };
                }
                let record = invocation.record();
                created(&record, &self.invocation(&record, urls, true), urls)
            }
            Endpoint::ListActionInvocations { thing, action } => {
                let list: Vec<Value> = invocations
                    .list_for(thing, action)
                    .iter()
                    .map(|i| self.invocation(&i.record(), urls, false))
                    .collect();
                render::json(StatusCode::OK, &Value::Array(list))
            }
            Endpoint::QueryAllActions { thing } => {
                let Some(handle) = self.runtime.thing(thing) else {
                    return self.error(HttpError::not_found());
                };
                let mut out = Map::new();
                for action in handle.actions() {
                    let mut records: Vec<InvocationRecord> = invocations
                        .list_for(thing, action.name())
                        .iter()
                        .map(|i| i.record())
                        .collect();
                    // The most recent first, as the WoT Profile wants.
                    records.sort_by_key(|record| std::cmp::Reverse(record.time_requested));
                    let list = records
                        .iter()
                        .map(|record| self.invocation(record, urls, false))
                        .collect();
                    out.insert(action.name().to_owned(), Value::Array(list));
                }
                render::json(StatusCode::OK, &Value::Object(out))
            }
            Endpoint::ListInvocations => {
                let list: Vec<Value> = invocations
                    .list()
                    .iter()
                    .map(|i| self.invocation(&i.record(), urls, false))
                    .collect();
                render::json(StatusCode::OK, &Value::Array(list))
            }
            Endpoint::GetInvocation => {
                let id = match self.parse_id(param.as_deref(), "id") {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                match invocations.get(id) {
                    Some(invocation) => render::json(
                        StatusCode::OK,
                        &self.invocation(&invocation.record(), urls, true),
                    ),
                    None => self.error(HttpError::Status(
                        StatusCode::NOT_FOUND,
                        CancelError::NotFound(id).to_string(),
                    )),
                }
            }
            Endpoint::CancelInvocation => {
                let id = match self.parse_id(param.as_deref(), "id") {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                match invocations.cancel(id) {
                    Ok(()) => self.written(StatusCode::OK),
                    Err(error @ CancelError::NotFound(_)) => {
                        self.error(HttpError::Status(StatusCode::NOT_FOUND, error.to_string()))
                    }
                    Err(error @ CancelError::NotCancellable(_)) => self.error(HttpError::Typed {
                        tetathing: StatusCode::SERVICE_UNAVAILABLE,
                        wot: StatusCode::CONFLICT,
                        name: "not-cancellable",
                        title: "Not cancellable",
                        detail: error.to_string(),
                    }),
                }
            }
            Endpoint::InvocationOutput => {
                let id = match self.parse_id(param.as_deref(), "id") {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(invocation) = invocations.get(id) else {
                    return self.error(HttpError::Status(
                        StatusCode::NOT_FOUND,
                        CancelError::NotFound(id).to_string(),
                    ));
                };
                // A Blob output is its data.
                if let Some(blob) = invocation.output_blob() {
                    return output::blob_response(blob).await;
                }
                // Reference bug B3 fixed: falsy outputs such as 0 are returned;
                // only a missing or null output is "no result".
                match invocation.output() {
                    Some(mut output) if !output.is_null() => {
                        output::resolve_blobs(&mut output, urls);
                        render::json(StatusCode::OK, &output)
                    }
                    _ => self.error(HttpError::Typed {
                        tetathing: StatusCode::SERVICE_UNAVAILABLE,
                        wot: StatusCode::NOT_FOUND,
                        name: "no-output",
                        title: "No output",
                        detail: "No result is available for this invocation".to_owned(),
                    }),
                }
            }
            Endpoint::DownloadBlob => {
                let id = match self.parse_id(param.as_deref(), "blob_id") {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                match BlobData::find(id) {
                    Some(data) => output::blob_response(data).await,
                    None => self.error(HttpError::Status(
                        StatusCode::NOT_FOUND,
                        "Blob not found".to_owned(),
                    )),
                }
            }
        }
    }

    /// The answer to a successful write, reset or cancellation: nothing in
    /// the `wot` profile; TetaThing's `status` and `null` otherwise.
    fn written(&self, tetathing: StatusCode) -> Response {
        if self.wot() {
            render::no_content()
        } else {
            render::json(tetathing, &Value::Null)
        }
    }

    /// An invocation as the profile shows it: TetaThing's JSON (`full` adds
    /// input, output, log and error), or the WoT Profile's `ActionStatus`.
    fn invocation(&self, record: &InvocationRecord, urls: &Urls, full: bool) -> Value {
        if self.wot() {
            let has_output = self
                .runtime
                .thing(&record.thing)
                .and_then(|t| t.action(&record.action))
                .is_none_or(|a| a.has_output());
            render::action_status(record, urls, has_output)
        } else {
            render::invocation(record, urls, full)
        }
    }

    /// `writemultipleproperties`: every value is checked before
    /// any is written, so an invalid request changes nothing. Names that
    /// aren't writable properties are validation errors.
    async fn write_multiple(&self, thing: &str, body: Value) -> Response {
        let Some(handle) = self.runtime.thing(thing) else {
            return self.error(HttpError::not_found());
        };
        let Value::Object(values) = body else {
            return self.error(HttpError::Invalid(&[ValidationIssue {
                kind: "dict_type".into(),
                loc: vec![LocItem::from("body")],
                msg: "Input should be a valid dictionary".into(),
                input: body,
                ctx: None,
            }]));
        };
        let mut issues = Vec::new();
        for (name, value) in &values {
            let issue = |kind: &str, msg: &str| ValidationIssue {
                kind: kind.into(),
                loc: vec![LocItem::from("body"), LocItem::from(name.as_str())],
                msg: msg.into(),
                input: value.clone(),
                ctx: None,
            };
            let Some(property) = handle.property(name) else {
                issues.push(issue("extra_forbidden", "Extra inputs are not permitted"));
                continue;
            };
            match property.validate(value) {
                Ok(()) => {}
                Err(PropertyError::ReadOnly(_)) => {
                    issues.push(issue("frozen_field", "Field is frozen"))
                }
                Err(PropertyError::Invalid(invalid)) => {
                    issues.extend(invalid.issues.into_iter().map(|mut issue| {
                        issue.loc.insert(1, LocItem::from(name.as_str()));
                        issue
                    }));
                }
                Err(other) => return self.property_error(thing, other),
            }
        }
        if !issues.is_empty() {
            return self.error(HttpError::Invalid(&issues));
        }
        for (name, value) in values {
            if let Some(property) = handle.property(&name)
                && let Err(error) = property.write(value).await
            {
                return self.property_error(thing, error);
            }
        }
        render::no_content()
    }

    /// A Thing's TD as served: with the request's base URL and a stable `id`.
    fn td(&self, thing: &str, urls: &Urls) -> Result<Value, String> {
        served_td(&self.runtime, thing, urls, &self.options)
    }

    /// Every served TD, for the directory.
    fn all_tds(&self, urls: &Urls) -> Result<Vec<Value>, String> {
        self.runtime
            .things()
            .map(|thing| self.td(thing.name(), urls))
            .collect()
    }

    /// A property failure: invalid values are the profile's validation
    /// error; the rest are problem details (409 or 503 for a busy global
    /// lock, 500 otherwise).
    fn property_error(&self, thing: &str, error: PropertyError) -> Response {
        match error {
            PropertyError::Invalid(invalid) => self.error(HttpError::Invalid(&invalid.issues)),
            other => {
                tracing::error!(target: "wot::things", thing = %thing, "{other}");
                self.error(HttpError::Problem(other.problem()))
            }
        }
    }

    /// Reads a JSON body. An empty body is `null` (FastAPI treats both as
    /// missing); malformed JSON is FastAPI's `json_invalid` error, located
    /// at the character where parsing failed (in the `wot` profile, at the
    /// body, with the parser's message).
    #[allow(clippy::result_large_err)] // the error is the response to send
    async fn read_json(&self, body: Body) -> Result<Value, Response> {
        let bytes = match axum::body::to_bytes(body, BODY_LIMIT).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(self.error(HttpError::Status(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    error.to_string(),
                )));
            }
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
            let issue = if self.wot() {
                ValidationIssue {
                    kind: "json_invalid".into(),
                    loc: vec![LocItem::from("body")],
                    msg: format!("JSON decode error at character {position}: {message}"),
                    input: Value::Null,
                    ctx: None,
                }
            } else {
                let mut ctx = Map::new();
                ctx.insert("error".into(), json!(message));
                ValidationIssue {
                    kind: "json_invalid".into(),
                    loc: vec![LocItem::from("body"), LocItem::Index(position)],
                    msg: "JSON decode error".into(),
                    input: json!({}),
                    ctx: Some(ctx),
                }
            };
            self.error(HttpError::Invalid(&[issue]))
        })
    }

    /// Parses the ID in the path (the parameter `name`), or answers
    /// FastAPI's 422. In the `wot` profile, a malformed ID names nothing, so
    /// it is a 404.
    #[allow(clippy::result_large_err)]
    fn parse_id(&self, raw: Option<&str>, name: &str) -> Result<Uuid, Response> {
        let raw = raw.unwrap_or_default();
        Uuid::parse_str(raw).map_err(|error| {
            if self.wot() {
                let detail = if name == "blob_id" {
                    "Blob not found".to_owned()
                } else {
                    format!("No action invocation found with ID {raw}")
                };
                return self.error(HttpError::Status(StatusCode::NOT_FOUND, detail));
            }
            let mut ctx = Map::new();
            ctx.insert("error".into(), json!(error.to_string()));
            self.error(HttpError::Invalid(&[ValidationIssue {
                kind: "uuid_parsing".into(),
                loc: vec![LocItem::from("path"), LocItem::from(name)],
                msg: format!("Input should be a valid UUID, {error}"),
                input: json!(raw),
                ctx: Some(ctx),
            }]))
        })
    }
}

/// A 500 as a problem, for failures that shouldn't happen.
fn server_error(message: &str) -> Response {
    crate::problem::http_problem(StatusCode::INTERNAL_SERVER_ERROR, message)
}

/// The problem of an invocation that ended without one (it was cancelled).
fn cancelled_problem() -> ProblemDetails {
    ProblemDetails::teta_wot_things("InvocationCancelledError", "The action was cancelled.", 500)
}

/// A Thing's TD as served: with the base URL of the request, a stable `id`,
/// the descriptions of observation, events and links, the top-level and
/// invocation forms, and, in the `wot` profile, its `profile` and
/// synchronous actions.
pub(crate) fn served_td(
    runtime: &Runtime,
    thing: &str,
    urls: &Urls,
    options: &HttpOptions,
) -> Result<Value, String> {
    let handle = runtime
        .thing(thing)
        .ok_or_else(|| format!("no Thing named {thing}"))?;
    let wot = options.profile == WireProfile::Wot;
    let td_options = TdOptions {
        path: urls.thing_path(thing),
        base: Some(urls.base()),
        id: Some(td_id(&options.server_id, thing)),
        observation: true,
        websocket: Some(urls.websocket(thing)),
        links: true,
        top_level: true,
        invocations: Some(format!("{}/action_invocations/", urls.prefix)),
        synchronous_actions: wot,
        profiles: if wot {
            vec![HTTP_BASIC_PROFILE.to_owned(), HTTP_SSE_PROFILE.to_owned()]
        } else {
            Vec::new()
        },
        security: options.security.as_ref().map(security::scheme),
    };
    let td = handle
        .thing_description(&td_options)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(td).map_err(|e| e.to_string())
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
