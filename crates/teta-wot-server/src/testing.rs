//! An in-process test client (`testing` feature).
//!
//! [`TestClient`] sends requests straight to a server's router, without a
//! socket. Starting it starts
//! the Things; stopping it shuts them down. Requests carry `Host: testserver`
//! by default, so absolute URLs match the reference's fixtures.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use teta_wot_core::{Runtime, StartupError};
use tower::ServiceExt;

use crate::ThingServer;

/// A response received by a [`TestClient`].
#[derive(Debug, Clone)]
pub struct TestResponse {
    /// The status.
    pub status: StatusCode,
    /// The headers.
    pub headers: HeaderMap,
    /// The body.
    pub body: Bytes,
}

impl TestResponse {
    /// The body as JSON (`null` if it is empty or not JSON).
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The body as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// A header's value, if present and text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Sends requests to a [`ThingServer`] in-process.
pub struct TestClient {
    router: Router,
    runtime: std::sync::Arc<Runtime>,
    host: String,
    /// Headers sent with every request, such as credentials.
    headers: Vec<(String, String)>,
}

impl std::fmt::Debug for TestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestClient")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl TestClient {
    /// Starts the server's Things and returns a client for it.
    pub async fn start(server: ThingServer) -> Result<Self, StartupError> {
        server.runtime().start().await?;
        Ok(Self {
            router: server.router(),
            runtime: std::sync::Arc::clone(server.runtime()),
            host: "testserver".to_owned(),
            headers: Vec::new(),
        })
    }

    /// Uses another `Host` header (and so another base URL in responses).
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sends a header with every request, such as `Authorization`.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn builder(&self) -> http::request::Builder {
        let mut builder = Request::builder().header("host", &self.host);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        builder
    }

    /// The runtime behind the server.
    pub fn runtime(&self) -> &std::sync::Arc<Runtime> {
        &self.runtime
    }

    /// Sends a request with optional headers and body.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> TestResponse {
        let mut builder = self.builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder
            .body(body.map_or_else(Body::empty, Body::from))
            .expect("a valid test request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("the router is infallible");
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();
        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    /// `GET path`.
    pub async fn get(&self, path: &str) -> TestResponse {
        self.request(Method::GET, path, &[], None).await
    }

    /// `DELETE path`.
    pub async fn delete(&self, path: &str) -> TestResponse {
        self.request(Method::DELETE, path, &[], None).await
    }

    /// `PUT path` with a JSON body.
    pub async fn put_json(&self, path: &str, body: &Value) -> TestResponse {
        self.send_json(Method::PUT, path, Some(body)).await
    }

    /// `POST path` with a JSON body, or none.
    pub async fn post_json(&self, path: &str, body: Option<&Value>) -> TestResponse {
        self.send_json(Method::POST, path, body).await
    }

    async fn send_json(&self, method: Method, path: &str, body: Option<&Value>) -> TestResponse {
        match body {
            Some(body) => {
                let bytes = serde_json::to_vec(body).expect("JSON");
                self.request(
                    method,
                    path,
                    &[("content-type", "application/json")],
                    Some(bytes),
                )
                .await
            }
            None => self.request(method, path, &[], None).await,
        }
    }

    /// `GET path`, without reading the body: for long responses such as
    /// MJPEG streams, read with `http_body_util::BodyExt::frame`.
    pub async fn stream(&self, path: &str) -> axum::response::Response {
        let request = self
            .builder()
            .uri(path)
            .body(Body::empty())
            .expect("a valid test request");
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("the router is infallible")
    }

    /// Opens a server-sent events stream: `GET path` with
    /// `Accept: text/event-stream`, in-process.
    pub async fn events(&self, path: &str) -> TestEventStream {
        let request = self
            .builder()
            .uri(path)
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .expect("a valid test request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("the router is infallible");
        let (parts, body) = response.into_parts();
        TestEventStream {
            status: parts.status,
            headers: parts.headers,
            body,
            buffer: String::new(),
        }
    }

    /// Opens a WebSocket (`websocket_connect`). A WebSocket
    /// needs a real connection, so the router is served on a loopback port
    /// for as long as the socket is open; requests carry this client's
    /// `Host`.
    pub async fn websocket(&self, path: &str) -> TestWebSocket {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a loopback port");
        let addr = listener.local_addr().expect("a bound address");
        let server = tokio::spawn(axum::serve(listener, self.router.clone()).into_future());
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("the loopback server accepts");
        let url = format!("ws://{}{path}", self.host);
        let mut request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url)
                .expect("a valid WebSocket URL");
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                http::HeaderName::from_bytes(name.as_bytes()),
                http::HeaderValue::from_str(value),
            ) {
                request.headers_mut().insert(name, value);
            }
        }
        let (socket, _) = tokio_tungstenite::client_async(request, stream)
            .await
            .expect("the WebSocket handshake succeeds");
        TestWebSocket { socket, server }
    }

    /// Cancels running invocations and stops the Things.
    pub async fn stop(self) {
        self.runtime.shutdown(Duration::from_secs(1)).await;
    }
}

/// How long [`TestEventStream`] and [`TestWebSocket`] wait for a message
/// before failing the test.
pub const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// A server-sent events stream opened by [`TestClient::events`].
pub struct TestEventStream {
    /// The response's status.
    pub status: StatusCode,
    /// The response's headers.
    pub headers: HeaderMap,
    body: Body,
    buffer: String,
}

impl std::fmt::Debug for TestEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEventStream")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl TestEventStream {
    /// A header's value, if present and text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The next event's `data`, as JSON (a string if it isn't JSON), or
    /// `None` when the stream ends. Comments (keep-alives) are skipped.
    /// Panics after [`RECEIVE_TIMEOUT`].
    pub async fn next(&mut self) -> Option<Value> {
        self.next_message().await.map(|message| message.data)
    }

    /// The next event, with its name and ID: as [`next`](Self::next).
    pub async fn next_message(&mut self) -> Option<TestSseMessage> {
        tokio::time::timeout(RECEIVE_TIMEOUT, self.next_event())
            .await
            .expect("no server-sent event within the timeout")
    }

    /// The next event's `data` if one arrives within `timeout`.
    pub async fn next_within(&mut self, timeout: Duration) -> Option<Value> {
        tokio::time::timeout(timeout, self.next_event())
            .await
            .ok()
            .flatten()
            .map(|message| message.data)
    }

    async fn next_event(&mut self) -> Option<TestSseMessage> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let event: String = self.buffer.drain(..end + 2).collect();
                let data: Vec<&str> = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(|data| data.strip_prefix(' ').unwrap_or(data))
                    .collect();
                if data.is_empty() {
                    continue;
                }
                let field = |name: &str| {
                    event
                        .lines()
                        .find_map(|line| line.strip_prefix(name))
                        .map(|value| value.strip_prefix(' ').unwrap_or(value).to_owned())
                };
                let data = data.join("\n");
                return Some(TestSseMessage {
                    event: field("event:"),
                    id: field("id:"),
                    data: serde_json::from_str(&data).unwrap_or(Value::String(data)),
                });
            }
            let frame = self.body.frame().await?.ok()?;
            if let Ok(bytes) = frame.into_data() {
                self.buffer
                    .push_str(&String::from_utf8_lossy(&bytes).replace("\r\n", "\n"));
            }
        }
    }
}

/// A server-sent event received by [`TestEventStream::next_message`].
#[derive(Debug, Clone, PartialEq)]
pub struct TestSseMessage {
    /// The event's name (`event:`), if it has one.
    pub event: Option<String>,
    /// The event's ID (`id:`), if it has one.
    pub id: Option<String>,
    /// The event's data, as JSON (a string if it isn't JSON).
    pub data: Value,
}

/// A WebSocket opened by [`TestClient::websocket`].
pub struct TestWebSocket {
    socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl std::fmt::Debug for TestWebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestWebSocket").finish_non_exhaustive()
    }
}

impl Drop for TestWebSocket {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// How a [`TestWebSocket`] was closed by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestClose {
    /// The close code (1005 if the frame had none).
    pub code: u16,
    /// The reason.
    pub reason: String,
}

impl TestWebSocket {
    /// Sends a JSON text message.
    pub async fn send_json(&mut self, value: &Value) {
        self.send_text(&value.to_string()).await;
    }

    /// Sends a text message.
    pub async fn send_text(&mut self, text: &str) {
        use futures_util::SinkExt;
        self.socket
            .send(tokio_tungstenite::tungstenite::Message::text(text))
            .await
            .expect("the WebSocket is open");
    }

    /// Receives a JSON text message. Panics if the socket closes, or after
    /// [`RECEIVE_TIMEOUT`].
    pub async fn receive_json(&mut self) -> Value {
        match tokio::time::timeout(RECEIVE_TIMEOUT, self.receive()).await {
            Ok(Ok(value)) => value,
            Ok(Err(close)) => panic!("the WebSocket closed: {close:?}"),
            Err(_) => panic!("no WebSocket message within the timeout"),
        }
    }

    /// Receives a JSON text message if one arrives within `timeout`.
    pub async fn receive_json_within(&mut self, timeout: Duration) -> Option<Value> {
        tokio::time::timeout(timeout, self.receive())
            .await
            .ok()
            .and_then(Result::ok)
    }

    /// Waits for the server to close the socket, skipping messages.
    pub async fn closed(&mut self) -> TestClose {
        tokio::time::timeout(RECEIVE_TIMEOUT, async {
            loop {
                if let Err(close) = self.receive().await {
                    return close;
                }
            }
        })
        .await
        .expect("the WebSocket didn't close within the timeout")
    }

    /// Closes the socket with code 1000.
    pub async fn close(mut self) {
        let _ = self.socket.close(None).await;
    }

    async fn receive(&mut self) -> Result<Value, TestClose> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Ok(serde_json::from_str(text.as_str())
                        .unwrap_or_else(|_| Value::String(text.as_str().to_owned())));
                }
                Some(Ok(Message::Close(frame))) => {
                    return Err(frame.map_or(
                        TestClose {
                            code: 1005,
                            reason: String::new(),
                        },
                        |frame| TestClose {
                            code: frame.code.into(),
                            reason: frame.reason.as_str().to_owned(),
                        },
                    ));
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    return Err(TestClose {
                        code: 1006,
                        reason: String::new(),
                    });
                }
            }
        }
    }
}

/// Testing one Thing (`create_thing_without_server`): it runs on
/// a small server with a temporary settings folder, and its slots are
/// filled by stand-in Things (fakes that implement the same interface, or
/// real Things with test configurations) added with [`HarnessBuilder::thing`].
///
/// ```ignore
/// let harness = Harness::builder("autofocus", Autofocus::default())
///     .thing("stage", FakeStage::default())
///     .thing("camera", FakeCamera::default())
///     .start()
///     .await?;
/// let autofocus = harness.thing(); // a ThingRef<Autofocus>
/// harness.client().post_json("/autofocus/focus", None).await;
/// ```
pub struct Harness<T> {
    client: TestClient,
    thing: teta_wot_core::ThingRef<T>,
    settings: Option<tempfile::TempDir>,
}

/// Builds a [`Harness`].
pub struct HarnessBuilder<T> {
    builder: crate::ThingServerBuilder,
    name: String,
    settings: Option<tempfile::TempDir>,
    temporary_settings: bool,
    _thing: std::marker::PhantomData<fn() -> T>,
}

/// A harness can't start.
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// The server can't be built (for example, a slot has no stand-in).
    #[error(transparent)]
    Build(#[from] crate::ServerBuildError),
    /// A Thing failed to start.
    #[error(transparent)]
    Startup(#[from] StartupError),
    /// The temporary settings folder can't be made.
    #[error("couldn't make a temporary settings folder: {0}")]
    Settings(std::io::Error),
}

impl<T: teta_wot_core::Thing> Harness<T> {
    /// A harness for `thing`, served as `name`.
    pub fn builder(name: impl Into<String>, thing: T) -> HarnessBuilder<T> {
        let name = name.into();
        HarnessBuilder {
            builder: ThingServer::builder().thing(name.clone(), thing),
            name,
            settings: None,
            temporary_settings: true,
            _thing: std::marker::PhantomData,
        }
    }

    /// The Thing under test.
    pub fn thing(&self) -> &teta_wot_core::ThingRef<T> {
        &self.thing
    }

    /// An HTTP client for the server.
    pub fn client(&self) -> &TestClient {
        &self.client
    }

    /// The temporary settings folder, if the harness has one.
    pub fn settings_folder(&self) -> Option<&std::path::Path> {
        self.settings.as_ref().map(tempfile::TempDir::path)
    }

    /// Stops the Things. The temporary settings folder is deleted.
    pub async fn stop(self) {
        self.client.stop().await;
    }
}

impl<T: teta_wot_core::Thing> HarnessBuilder<T> {
    /// Adds another Thing, for example a stand-in for a slot.
    #[must_use]
    pub fn thing<U: teta_wot_core::Thing>(mut self, name: impl Into<String>, thing: U) -> Self {
        self.builder = self.builder.thing(name, thing);
        self
    }

    /// Registers a service.
    #[must_use]
    pub fn service<S: ?Sized + Send + Sync + 'static>(
        mut self,
        service: std::sync::Arc<S>,
    ) -> Self {
        self.builder = self.builder.service(service);
        self
    }

    /// Configures the slots of the Thing under test.
    #[must_use]
    pub fn slots(
        mut self,
        slots: impl IntoIterator<Item = (String, teta_wot_core::SlotSelection)>,
    ) -> Self {
        let name = self.name.clone();
        self.builder = self.builder.thing_slots(name, slots);
        self
    }

    /// Sets the application configuration.
    #[must_use]
    pub fn application_config(mut self, config: Value) -> Self {
        self.builder = self.builder.application_config(config);
        self
    }

    /// Enables the global lock.
    #[must_use]
    pub fn global_lock(mut self, enabled: bool) -> Self {
        self.builder = self.builder.global_lock(enabled);
        self
    }

    /// Uses an existing settings folder instead of a temporary one, for
    /// example to test loading a prepared settings file.
    #[must_use]
    pub fn settings_folder(mut self, folder: impl Into<std::path::PathBuf>) -> Self {
        self.builder = self.builder.settings_folder(folder);
        self.temporary_settings = false;
        self
    }

    /// Builds the server and starts the Things.
    pub async fn start(mut self) -> Result<Harness<T>, HarnessError> {
        if self.temporary_settings {
            let folder = tempfile::tempdir().map_err(HarnessError::Settings)?;
            self.builder = self.builder.settings_folder(folder.path());
            self.settings = Some(folder);
        }
        let server = self.builder.build()?;
        let thing = server
            .runtime()
            .thing_ref::<T>(&self.name)
            .expect("the Thing under test was added as a T");
        Ok(Harness {
            client: TestClient::start(server).await?,
            thing,
            settings: self.settings,
        })
    }
}
