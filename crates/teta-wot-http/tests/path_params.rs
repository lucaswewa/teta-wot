//! Path parameters of custom endpoints: matching, precedence, decoding,
//! conflicts, and how OpenAPI describes them.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use serde_json::json;
use teta_wot_core::{Runtime, Thing, ThingDefinition};
use teta_wot_http::{Endpoint, HttpOptions, PathParams, RouteError, openapi, router};
use tower::ServiceExt;

/// Which endpoint answered, and the parameters it got.
fn describe(which: &str, params: &PathParams) -> String {
    let params: Vec<String> = params.iter().map(|(n, v)| format!("{n}={v}")).collect();
    format!("{which} {}", params.join(" "))
        .trim_end()
        .to_owned()
}

/// A Thing whose endpoints overlap, so that precedence decides.
struct Files;

impl Thing for Files {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Files")
            .endpoint(Endpoint::new("get", "files/{*rest}", |_: Arc<Self>| {
                |p: PathParams| async move { describe("wildcard", &p) }
            }))
            .endpoint(Endpoint::new("get", "files/{name}", |_: Arc<Self>| {
                |p: PathParams| async move { describe("param", &p) }
            }))
            .endpoint(Endpoint::new("get", "files/readme", |_: Arc<Self>| {
                |p: PathParams| async move { describe("literal", &p) }
            }))
            .endpoint(Endpoint::new(
                "get",
                "tiles/{level}/{tile}",
                |_: Arc<Self>| {
                    |p: PathParams| async move {
                        let level: Option<u32> = p.parse("level");
                        format!("{} (level {level:?})", describe("tile", &p))
                    }
                },
            ))
    }
}

async fn get(path: &str) -> (StatusCode, String) {
    let runtime = Arc::new(Runtime::builder().thing("files", Files).build().unwrap());
    let app = router(runtime, HttpOptions::default()).unwrap();
    let request = Request::get(path).body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn the_most_specific_endpoint_answers_with_its_parameters() {
    for (path, answer) in [
        ("/files/files/readme", "literal"),
        ("/files/files/notes.txt", "param name=notes.txt"),
        ("/files/files/a/b/c.txt", "wildcard rest=a/b/c.txt"),
        (
            "/files/tiles/3/0_1.jpeg",
            "tile level=3 tile=0_1.jpeg (level Some(3))",
        ),
        ("/files/tiles/x/y", "tile level=x tile=y (level None)"),
    ] {
        assert_eq!(
            get(path).await,
            (StatusCode::OK, answer.to_owned()),
            "{path}"
        );
    }
}

#[tokio::test]
async fn values_are_percent_decoded_and_empty_ones_match_nothing() {
    assert_eq!(
        get("/files/files/a%20b%2Fc").await,
        (StatusCode::OK, "param name=a b/c".to_owned())
    );
    assert_eq!(get("/files/files/%FF").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(get("/files/tiles/3").await.0, StatusCode::NOT_FOUND);
    assert_eq!(get("/files/tiles//1").await.0, StatusCode::NOT_FOUND);
}

/// Two endpoints whose paths differ only in their parameters' names.
struct Ambiguous;

impl Thing for Ambiguous {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Ambiguous")
            .endpoint(Endpoint::new("get", "x/{a}", |_: Arc<Self>| {
                || async { "a" }
            }))
            .endpoint(Endpoint::new("get", "x/{b}", |_: Arc<Self>| {
                || async { "b" }
            }))
            .endpoint(Endpoint::new("put", "x/{c}", |_: Arc<Self>| {
                || async { "c" }
            }))
    }
}

#[test]
fn endpoints_of_the_same_shape_are_rejected_when_the_server_is_built() {
    let runtime = Runtime::builder().thing("t", Ambiguous).build().unwrap();
    let error = router(Arc::new(runtime), HttpOptions::default()).unwrap_err();
    assert!(
        matches!(&error, RouteError::EndpointConflict { route, .. } if route == "GET /t/x/{b}"),
        "{error}"
    );
}

/// An endpoint whose path has a parameter inside a segment.
struct Malformed;

impl Thing for Malformed {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Malformed").endpoint(Endpoint::new(
            "get",
            "{dataset}_files",
            |_: Arc<Self>| || async { "" },
        ))
    }
}

#[test]
fn malformed_paths_are_rejected_when_the_runtime_is_built() {
    let error = Runtime::builder()
        .thing("m", Malformed)
        .build()
        .unwrap_err();
    assert!(error.to_string().contains("whole segment"), "{error}");
}

#[test]
fn openapi_lists_path_parameters() {
    let runtime = Runtime::builder().thing("files", Files).build().unwrap();
    let document = openapi(&runtime, &HttpOptions::default());
    let parameter = |name: &str| json!({"name": name, "in": "path", "required": true, "schema": {"type": "string"}});
    let paths = &document["paths"];
    assert_eq!(
        paths["/files/tiles/{level}/{tile}"]["get"]["parameters"],
        json!([parameter("level"), parameter("tile")])
    );
    let mut rest = parameter("rest");
    rest["description"] = json!("The rest of the path: one or more segments, slashes included.");
    assert_eq!(
        paths["/files/files/{rest}"]["get"]["parameters"],
        json!([rest])
    );
    assert!(paths["/files/files/readme"]["get"]["parameters"].is_null());
}
