//! CORS behavior tested through the public router.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use teta_wot_core::Runtime;
use teta_wot_http::{HttpOptions, router};
use tower::ServiceExt;

async fn request(method: Method, path: &str, headers: &[(&str, &str)]) -> axum::response::Response {
    let app = router(
        Arc::new(Runtime::builder().build().unwrap()),
        HttpOptions::default(),
    )
    .unwrap();
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn responses_without_origin_vary_by_origin_without_cors_credentials() {
    for (path, status) in [
        ("/things/", StatusCode::OK),
        ("/unknown", StatusCode::NOT_FOUND),
    ] {
        let response = request(Method::GET, path, &[]).await;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["vary"], "Origin");
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-credentials")
        );
    }
}

#[tokio::test]
async fn origin_and_credentials_are_mirrored_on_success_errors_and_redirects() {
    for (path, status) in [
        ("/things/", StatusCode::OK),
        ("/unknown", StatusCode::NOT_FOUND),
        ("/things", StatusCode::TEMPORARY_REDIRECT),
    ] {
        let response = request(Method::GET, path, &[("origin", "https://client.example")]).await;
        assert_eq!(response.status(), status);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "https://client.example"
        );
        assert_eq!(
            response.headers()["access-control-allow-credentials"],
            "true"
        );
        assert_eq!(response.headers()["vary"], "Origin");
    }
}

#[tokio::test]
async fn preflight_on_unknown_paths_allows_methods_and_echoes_requested_headers() {
    let response = request(
        Method::OPTIONS,
        "/not-a-route",
        &[
            ("origin", "https://client.example"),
            ("access-control-request-method", "PUT"),
            ("access-control-request-headers", "X-Custom, Content-Type"),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    assert_eq!(
        headers["access-control-allow-origin"],
        "https://client.example"
    );
    assert_eq!(headers["access-control-allow-credentials"], "true");
    assert_eq!(
        headers["access-control-allow-headers"],
        "X-Custom, Content-Type"
    );
    assert_eq!(
        headers["access-control-allow-methods"],
        "DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY"
    );
    assert_eq!(headers["access-control-max-age"], "600");
    assert_eq!(
        headers["vary"],
        "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network"
    );
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap(), "OK");
}

#[tokio::test]
async fn preflight_without_requested_headers_omits_allow_headers() {
    let response = request(
        Method::OPTIONS,
        "/things/",
        &[("origin", "null"), ("access-control-request-method", "GET")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["access-control-allow-origin"], "null");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-headers")
    );
}

#[tokio::test]
async fn options_without_both_preflight_headers_uses_normal_dispatch() {
    for headers in [
        vec![],
        vec![("origin", "https://client.example")],
        vec![("access-control-request-method", "GET")],
    ] {
        let response = request(Method::OPTIONS, "/things/", &headers).await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()["allow"], "GET");
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-methods")
        );
    }
}
