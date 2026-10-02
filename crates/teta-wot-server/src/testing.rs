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
        })
    }

    /// Uses another `Host` header (and so another base URL in responses).
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
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
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", &self.host);
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

    /// Cancels running invocations and stops the Things.
    pub async fn stop(self) {
        self.runtime.shutdown(Duration::from_secs(1)).await;
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
