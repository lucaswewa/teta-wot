//! End-to-end HTTP binding tests using in-process requests without sockets.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use serde_json::{Value, json};
use teta_wot_core::{
    Action, ActionCtx, ActionError, Constraints, DataProperty, FunctionalProperty, Invocation,
    InvocationStatus, NoInput, Prop, PropertyError, Runtime, Thing, ThingDefinition,
};
use teta_wot_http::{HttpOptions, router, td_id};
use tokio::sync::Notify;
use tower::ServiceExt;
use uuid::Uuid;

struct Counter {
    count: Prop<i64>,
    readonly: Prop<i64>,
    optional: Prop<Option<i64>>,
    started: Notify,
}
impl Default for Counter {
    fn default() -> Self {
        Self {
            count: Prop::new(2).with_constraints(Constraints::new().ge(0).le(10)),
            readonly: Prop::new(5),
            optional: Prop::new(Some(1)),
            started: Notify::new(),
        }
    }
}
impl Thing for Counter {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Counter")
            .description("Counts through HTTP.")
            .property(
                "count",
                DataProperty::new(|t: &Self| &t.count).unit("items"),
            )
            .property(
                "readonly",
                DataProperty::new(|t: &Self| &t.readonly).read_only(),
            )
            .property("optional", DataProperty::new(|t: &Self| &t.optional))
            .property(
                "computed",
                FunctionalProperty::getter(|t: Arc<Self>| async move { Ok(t.count.get()) })
                    .setter(|t, value| async move { t.count.set(value) }),
            )
            .property(
                "broken",
                FunctionalProperty::getter(|_: Arc<Self>| async {
                    Err::<i64, _>(PropertyError::failed(std::io::Error::other("read failed")))
                }),
            )
            .property(
                "unlocked",
                FunctionalProperty::getter(|t: Arc<Self>| async move { Ok(t.count.get()) })
                    .setter(|t, value| async move { t.count.set(value) })
                    .global_lock(false),
            )
            .action(
                "increment",
                Action::new(|t: Arc<Self>, _: ActionCtx, input: i64| async move {
                    t.count.update(|n| *n += input)?;
                    tracing::info!("incremented counter");
                    Ok::<_, ActionError>(t.count.get())
                })
                .retention(Duration::from_secs(5)),
            )
            .action(
                "echo",
                Action::new(|_: Arc<Self>, _: ActionCtx, input: Value| async move {
                    Ok::<_, ActionError>(input)
                }),
            )
            .action(
                "noop",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    Ok::<_, ActionError>(())
                }),
            )
            .action(
                "wait",
                Action::new(|t: Arc<Self>, ctx: ActionCtx, _: NoInput| async move {
                    t.started.notify_one();
                    Err::<(), ActionError>(ctx.cancelled().await.into())
                }),
            )
            .action(
                "failed",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    Err::<(), _>(ActionError::handled("action failed"))
                }),
            )
    }
}

struct Fixture {
    app: Router,
    runtime: Arc<Runtime>,
    thing: Arc<Counter>,
    prefix: String,
}
impl Fixture {
    fn new(prefix: &str, locked: bool) -> Self {
        let thing = Arc::new(Counter::default());
        let runtime = Arc::new(
            Runtime::builder()
                .global_lock(locked)
                .global_lock_timeout(Duration::from_millis(10))
                .thing_arc("counter", thing.clone())
                .build()
                .unwrap(),
        );
        let app = router(
            runtime.clone(),
            HttpOptions {
                api_prefix: prefix.into(),
                server_id: "test-server".into(),
            },
        )
        .unwrap();
        Self {
            app,
            runtime,
            thing,
            prefix: prefix.into(),
        }
    }
    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Response {
        let body = body.map_or_else(Body::empty, |value| {
            Body::from(serde_json::to_vec(&value).unwrap())
        });
        self.raw(
            method,
            &format!("{}{path}", self.prefix),
            &[("host", "testserver")],
            body,
        )
        .await
    }
    async fn raw(
        &self,
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
        body: Body,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        Response {
            status: parts.status,
            headers: parts.headers,
            body: to_bytes(body, 1024 * 1024).await.unwrap(),
        }
    }
    async fn invoke(&self, action: &str, input: Option<Value>) -> (Response, Arc<Invocation>) {
        let response = self
            .send(Method::POST, &format!("/counter/{action}"), input)
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.json());
        let id = Uuid::parse_str(response.json()["id"].as_str().unwrap()).unwrap();
        let invocation = self.runtime.invocations().get(id).unwrap();
        (response, invocation)
    }
}
struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}
impl Response {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    fn assert_json(&self, status: StatusCode, body: Value) {
        assert_eq!(self.status, status);
        assert_eq!(self.headers["content-type"], "application/json");
        assert_eq!(self.json(), body);
    }
}

#[tokio::test]
async fn property_reads_writes_coercion_and_reset_share_the_runtime_cell() {
    let fixture = Fixture::new("", false);
    fixture
        .send(Method::GET, "/counter/count", None)
        .await
        .assert_json(StatusCode::OK, json!(2));
    fixture
        .send(Method::PUT, "/counter/count", Some(json!("6")))
        .await
        .assert_json(StatusCode::CREATED, Value::Null);
    assert_eq!(fixture.thing.count.get(), 6);
    fixture
        .send(Method::GET, "/counter/count", None)
        .await
        .assert_json(StatusCode::OK, json!(6));
    fixture
        .send(Method::POST, "/counter/count/reset", None)
        .await
        .assert_json(StatusCode::OK, Value::Null);
    assert_eq!(fixture.thing.count.get(), 2);
    fixture
        .send(Method::PUT, "/counter/computed", Some(json!(4)))
        .await
        .assert_json(StatusCode::CREATED, Value::Null);
    fixture
        .send(Method::GET, "/counter/computed", None)
        .await
        .assert_json(StatusCode::OK, json!(4));
}

#[tokio::test]
async fn readonly_and_nonresettable_properties_have_no_mutating_routes() {
    let fixture = Fixture::new("", false);
    fixture
        .send(Method::GET, "/counter/readonly", None)
        .await
        .assert_json(StatusCode::OK, json!(5));
    let response = fixture
        .send(Method::PUT, "/counter/readonly", Some(json!(9)))
        .await;
    response.assert_json(
        StatusCode::METHOD_NOT_ALLOWED,
        json!({"detail":"Method Not Allowed"}),
    );
    assert_eq!(response.headers["allow"], "GET");
    for path in ["/counter/readonly/reset", "/counter/computed/reset"] {
        fixture
            .send(Method::POST, path, None)
            .await
            .assert_json(StatusCode::NOT_FOUND, json!({"detail":"Not Found"}));
    }
    assert_eq!(fixture.thing.readonly.get(), 5);
}

#[tokio::test]
async fn invalid_property_values_return_422_without_changing_the_cell() {
    let fixture = Fixture::new("", false);
    for (input, kind) in [
        (json!(11), "less_than_equal"),
        (json!("bad"), "int_parsing"),
        (Value::Null, "missing"),
    ] {
        let response = fixture
            .send(Method::PUT, "/counter/count", Some(input.clone()))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        let body = response.json();
        assert_eq!(body["detail"].as_array().unwrap().len(), 1);
        assert_eq!(body["detail"][0]["type"], kind);
        assert_eq!(body["detail"][0]["loc"], json!(["body"]));
        assert_eq!(body["detail"][0]["input"], input);
        assert_eq!(fixture.thing.count.get(), 2);
    }
    let response = fixture
        .send(Method::PUT, "/counter/optional", Some(Value::Null))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json()["detail"][0]["type"], "missing");
}

#[tokio::test]
async fn missing_and_malformed_bodies_have_distinct_validation_errors() {
    let fixture = Fixture::new("", false);
    let missing = fixture.send(Method::PUT, "/counter/count", None).await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.json()["detail"][0]["type"], "missing");
    let malformed = fixture
        .raw(
            Method::PUT,
            "/counter/count",
            &[],
            Body::from("{\n  \"x\":\n}"),
        )
        .await;
    assert_eq!(malformed.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = malformed.json();
    assert_eq!(body["detail"][0]["type"], "json_invalid");
    assert_eq!(body["detail"][0]["loc"], json!(["body", 9]));
    assert_eq!(body["detail"][0]["msg"], "JSON decode error");
    assert_eq!(body["detail"][0]["input"], json!({}));
    assert!(
        body["detail"][0]["ctx"]["error"]
            .as_str()
            .unwrap()
            .contains("expected value")
    );
    assert_eq!(fixture.thing.count.get(), 2);
}

#[tokio::test]
async fn oversized_request_bodies_return_413_before_mutating() {
    let fixture = Fixture::new("", false);
    let response = fixture
        .raw(
            Method::PUT,
            "/counter/count",
            &[],
            Body::from(vec![b' '; 64 * 1024 * 1024 + 1]),
        )
        .await;
    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(response.json()["detail"].is_string());
    assert_eq!(fixture.thing.count.get(), 2);
}

#[tokio::test]
async fn failed_property_getters_return_problem_details() {
    let fixture = Fixture::new("", false);
    fixture.send(Method::GET, "/counter/broken", None).await.assert_json(StatusCode::INTERNAL_SERVER_ERROR,
        json!({"detail":"read failed", "type":null, "status":500, "title":"PropertyError", "instance":null}));
}

#[tokio::test(start_paused = true)]
async fn busy_global_lock_returns_409_for_writes_and_resets_but_not_reads_or_opt_out() {
    let fixture = Fixture::new("", true);
    let guard = fixture
        .runtime
        .global_lock()
        .unwrap()
        .try_acquire(Uuid::new_v4())
        .unwrap();
    for (method, path) in [
        (Method::PUT, "/counter/count"),
        (Method::POST, "/counter/count/reset"),
    ] {
        let response = fixture.send(method, path, Some(json!(4))).await;
        assert_eq!(response.status, StatusCode::CONFLICT);
        assert_eq!(response.json()["title"], "GlobalLockBusyError");
        assert_eq!(
            response.json()["type"],
            teta_wot_core::problem::teta_wot_exception_type("GlobalLockBusyError")
        );
    }
    fixture
        .send(Method::GET, "/counter/count", None)
        .await
        .assert_json(StatusCode::OK, json!(2));
    fixture
        .send(Method::PUT, "/counter/unlocked", Some(json!(4)))
        .await
        .assert_json(StatusCode::CREATED, Value::Null);
    drop(guard);
    fixture
        .send(Method::PUT, "/counter/count", Some(json!(6)))
        .await
        .assert_json(StatusCode::CREATED, Value::Null);
}

#[tokio::test]
async fn unknown_paths_wrong_methods_and_head_requests_return_expected_responses() {
    let fixture = Fixture::new("", false);
    for path in [
        "/",
        "/missing",
        "/counter/missing",
        "/action_invocations//output",
        "/counter/count/extra",
    ] {
        fixture
            .send(Method::GET, path, None)
            .await
            .assert_json(StatusCode::NOT_FOUND, json!({"detail":"Not Found"}));
    }
    for (method, path, allow) in [
        (Method::POST, "/counter/count", "PUT"),
        (Method::PUT, "/counter/increment", "POST"),
        (Method::GET, "/counter/count/reset", "POST"),
    ] {
        let response = fixture.send(method, path, None).await;
        response.assert_json(
            StatusCode::METHOD_NOT_ALLOWED,
            json!({"detail":"Method Not Allowed"}),
        );
        assert_eq!(response.headers["allow"], allow);
    }
    for (path, status) in [
        ("/counter/", StatusCode::METHOD_NOT_ALLOWED),
        ("/missing", StatusCode::NOT_FOUND),
    ] {
        let response = fixture.send(Method::HEAD, path, None).await;
        assert_eq!(response.status, status);
        assert!(response.body.is_empty());
    }
}

#[tokio::test]
async fn trailing_slash_redirects_preserve_queries_and_prefixes() {
    let fixture = Fixture::new("/api/v1", false);
    for (path, target) in [
        ("/counter?x=1", "/counter/?x=1"),
        ("/counter/count/?x=1", "/counter/count?x=1"),
        ("/action_invocations/", "/action_invocations"),
    ] {
        let response = fixture.send(Method::GET, path, None).await;
        assert_eq!(response.status, StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers["location"],
            format!("http://testserver/api/v1{target}")
        );
        assert!(response.body.is_empty());
    }
    fixture
        .raw(Method::GET, "/counter/count", &[], Body::empty())
        .await
        .assert_json(StatusCode::NOT_FOUND, json!({"detail":"Not Found"}));
    fixture
        .send(Method::GET, "/counter/count", None)
        .await
        .assert_json(StatusCode::OK, json!(2));
}

#[tokio::test]
async fn thing_descriptions_and_indexes_use_request_urls_and_stable_ids() {
    let fixture = Fixture::new("/api", false);
    let response = fixture.send(Method::GET, "/counter/", None).await;
    assert_eq!(response.status, StatusCode::OK);
    let td = response.json();
    assert_eq!(td["title"], "Counter");
    assert_eq!(td["description"], "Counts through HTTP.");
    assert_eq!(td["base"], "http://testserver/");
    assert_eq!(td["id"], td_id("test-server", "counter"));
    assert_eq!(
        td["properties"]["count"]["forms"][0]["href"],
        "/api/counter/count"
    );
    assert_eq!(
        td["actions"]["increment"]["forms"][0]["href"],
        "/api/counter/increment"
    );
    fixture
        .send(Method::GET, "/things/", None)
        .await
        .assert_json(
            StatusCode::OK,
            json!({"counter":"http://testserver/api/counter/"}),
        );
    fixture
        .send(Method::GET, "/thing_descriptions/", None)
        .await
        .assert_json(StatusCode::OK, json!({"counter":td}));
    let other = fixture
        .raw(
            Method::GET,
            "https://uri.example/api/counter/",
            &[("host", "header.example:8443")],
            Body::empty(),
        )
        .await
        .json();
    assert_eq!(other["base"], "https://header.example:8443/");
    assert_eq!(other["id"], td["id"]);
    let authority = fixture
        .raw(
            Method::GET,
            "https://uri.example/api/things/",
            &[],
            Body::empty(),
        )
        .await;
    authority.assert_json(
        StatusCode::OK,
        json!({"counter":"https://uri.example/api/counter/"}),
    );
    let fallback = fixture
        .raw(Method::GET, "/api/things/", &[], Body::empty())
        .await;
    fallback.assert_json(
        StatusCode::OK,
        json!({"counter":"http://localhost/api/counter/"}),
    );
}

#[tokio::test]
async fn empty_runtime_indexes_return_empty_objects_and_lists() {
    let app = router(
        Arc::new(Runtime::builder().build().unwrap()),
        HttpOptions::default(),
    )
    .unwrap();
    for (path, expected) in [
        ("/things/", json!({})),
        ("/thing_descriptions/", json!({})),
        ("/action_invocations", json!([])),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(response.into_body(), 1024).await.unwrap())
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn action_creation_and_get_render_full_records_and_links() {
    teta_wot_core::testing::init_tracing();
    let fixture = Fixture::new("/api", false);
    let (created, invocation) = fixture.invoke("increment", Some(json!("3"))).await;
    let href = format!(
        "http://testserver/api/action_invocations/{}",
        invocation.id()
    );
    assert_eq!(created.headers["location"], href);
    assert_eq!(created.json()["href"], href);
    assert_eq!(created.json()["action"], "/api/counter/increment");
    assert_eq!(created.json()["input"], 3);
    assert_eq!(created.json()["status"], "pending");
    assert_eq!(created.json()["output"], Value::Null);
    assert_eq!(invocation.wait().await, InvocationStatus::Completed);
    let response = fixture
        .send(
            Method::GET,
            &format!("/action_invocations/{}", invocation.id()),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let record = response.json();
    assert_eq!(record["status"], "completed");
    assert_eq!(record["output"], 5);
    assert_eq!(record["error"], Value::Null);
    assert_eq!(
        record["links"],
        json!([
            {"href":href, "type":null, "rel":"self", "anchor":null},
            {"href":format!("{href}/output"), "type":null, "rel":"output", "anchor":null}
        ])
    );
    assert!(
        record["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["message"] == "incremented counter")
    );
    for key in ["timeRequested", "timeStarted", "timeCompleted"] {
        chrono::NaiveDateTime::parse_from_str(
            record[key].as_str().unwrap(),
            "%Y-%m-%dT%H:%M:%S%.f",
        )
        .unwrap();
    }
    let changed = fixture
        .raw(
            Method::GET,
            &format!("/api/action_invocations/{}", invocation.id()),
            &[("host", "other.example")],
            Body::empty(),
        )
        .await;
    assert_eq!(
        changed.json()["href"],
        format!(
            "http://other.example/api/action_invocations/{}",
            invocation.id()
        )
    );
}

#[tokio::test]
async fn invocation_lists_are_filtered_and_omit_full_record_fields() {
    let fixture = Fixture::new("", false);
    let (_, first) = fixture.invoke("increment", Some(json!(1))).await;
    first.wait().await;
    let (_, second) = fixture.invoke("noop", None).await;
    second.wait().await;
    let all = fixture
        .send(Method::GET, "/action_invocations", None)
        .await
        .json();
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(all[0]["id"], first.id().to_string());
    assert_eq!(all[1]["id"], second.id().to_string());
    for summary in all.as_array().unwrap() {
        for field in ["input", "output", "log", "error"] {
            assert!(summary.get(field).is_none());
        }
        assert_eq!(summary["status"], "completed");
    }
    let filtered = fixture
        .send(Method::GET, "/counter/increment", None)
        .await
        .json();
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["id"], first.id().to_string());
    fixture
        .send(Method::GET, "/counter/echo", None)
        .await
        .assert_json(StatusCode::OK, json!([]));
}

#[tokio::test]
async fn invalid_action_bodies_return_422_and_create_no_invocations() {
    let fixture = Fixture::new("", false);
    for input in [None, Some(Value::Null), Some(json!("bad"))] {
        let expected = if input.as_ref().is_some_and(Value::is_string) {
            "int_parsing"
        } else {
            "missing"
        };
        let response = fixture
            .send(Method::POST, "/counter/increment", input)
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.json()["detail"][0]["type"], expected);
        assert_eq!(response.json()["detail"][0]["loc"], json!(["body"]));
        assert!(!response.headers.contains_key("location"));
    }
    let response = fixture
        .raw(
            Method::POST,
            "/counter/increment",
            &[],
            Body::from("[broken"),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json()["detail"][0]["type"], "json_invalid");
    assert!(fixture.runtime.invocations().list().is_empty());
    assert_eq!(fixture.thing.count.get(), 2);
}

#[tokio::test]
async fn invocation_outputs_preserve_falsy_values_and_null_is_unavailable() {
    let fixture = Fixture::new("", false);
    for value in [
        json!(0),
        json!(false),
        json!(""),
        json!([]),
        json!({}),
        Value::Null,
    ] {
        let (_, invocation) = fixture.invoke("echo", Some(value.clone())).await;
        invocation.wait().await;
        let response = fixture
            .send(
                Method::GET,
                &format!("/action_invocations/{}/output", invocation.id()),
                None,
            )
            .await;
        if value.is_null() {
            response.assert_json(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"detail":"No result is available for this invocation"}),
            );
        } else {
            response.assert_json(StatusCode::OK, value);
        }
    }
}

#[tokio::test]
async fn cancellation_endpoints_cancel_running_actions_and_reject_finished_ones() {
    let fixture = Fixture::new("", false);
    let (_, invocation) = fixture.invoke("wait", None).await;
    fixture.thing.started.notified().await;
    let path = format!("/action_invocations/{}", invocation.id());
    fixture
        .send(Method::GET, &format!("{path}/output"), None)
        .await
        .assert_json(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"detail":"No result is available for this invocation"}),
        );
    fixture
        .send(Method::DELETE, &path, None)
        .await
        .assert_json(StatusCode::OK, Value::Null);
    assert_eq!(invocation.wait().await, InvocationStatus::Cancelled);
    let record = fixture.send(Method::GET, &path, None).await.json();
    assert_eq!(record["status"], "cancelled");
    assert_eq!(record["error"]["title"], "InvocationCancelledError");
    fixture.send(Method::DELETE, &path, None).await.assert_json(
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"detail":"The invocation is cancelled and may not be cancelled."}),
    );
    let (_, completed) = fixture.invoke("noop", None).await;
    completed.wait().await;
    fixture
        .send(
            Method::DELETE,
            &format!("/action_invocations/{}", completed.id()),
            None,
        )
        .await
        .assert_json(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"detail":"The invocation is completed and may not be cancelled."}),
        );
}

#[tokio::test]
async fn malformed_and_missing_invocation_ids_return_422_and_404() {
    let fixture = Fixture::new("", false);
    let missing = Uuid::new_v4();
    for (method, suffix) in [
        (Method::GET, ""),
        (Method::DELETE, ""),
        (Method::GET, "/output"),
    ] {
        let invalid = fixture
            .send(
                method.clone(),
                &format!("/action_invocations/bad-id{suffix}"),
                None,
            )
            .await;
        assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(invalid.json()["detail"][0]["type"], "uuid_parsing");
        assert_eq!(invalid.json()["detail"][0]["loc"], json!(["path", "id"]));
        assert_eq!(invalid.json()["detail"][0]["input"], "bad-id");
        fixture
            .send(
                method,
                &format!("/action_invocations/{missing}{suffix}"),
                None,
            )
            .await
            .assert_json(
                StatusCode::NOT_FOUND,
                json!({"detail":format!("No action invocation found with ID {missing}")}),
            );
    }
}

#[tokio::test]
async fn action_failure_is_reported_in_the_record_and_output_remains_unavailable() {
    let fixture = Fixture::new("", false);
    let (_, invocation) = fixture.invoke("failed", None).await;
    assert_eq!(invocation.wait().await, InvocationStatus::Error);
    let path = format!("/action_invocations/{}", invocation.id());
    let record = fixture.send(Method::GET, &path, None).await.json();
    assert_eq!(record["status"], "error");
    assert_eq!(record["error"]["detail"], "action failed");
    assert_eq!(record["error"]["title"], "InvocationError");
    fixture
        .send(Method::GET, &format!("{path}/output"), None)
        .await
        .assert_json(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"detail":"No result is available for this invocation"}),
        );
}

#[tokio::test(start_paused = true)]
async fn expired_invocations_are_removed_on_the_next_action_request() {
    let fixture = Fixture::new("", false);
    let (_, first) = fixture.invoke("increment", Some(json!(1))).await;
    first.wait().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    let (_, second) = fixture.invoke("noop", None).await;
    second.wait().await;
    let response = fixture
        .send(
            Method::GET,
            &format!("/action_invocations/{}", first.id()),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let list = fixture
        .send(Method::GET, "/action_invocations", None)
        .await
        .json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], second.id().to_string());
}
