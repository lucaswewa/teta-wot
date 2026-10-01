//! An in-process test client.
//!
//! [`TestClient`] sends requests straight to a server's router, without a
//! socket. Starting it starts the Things; stopping it shuts them down. Requests
//! carry `Host: testserver` by default, so absolute URLs match the reference's
//! fixtures.

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
