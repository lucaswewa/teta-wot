//! Regression tests for macros used through the `teta_wot` crate name.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

#[derive(Thing)]
struct Counter {
    #[property(default = 2, ge = 0, le = 10)]
    count: Prop<i64>,
    starts: AtomicUsize,
    stops: AtomicUsize,
}

#[thing_impl]
impl Counter {
    #[action]
    async fn add(&self, #[param(default = 2)] step: i64) -> Result<i64, ActionError> {
        self.count.update(|count| *count += step)?;
        Ok(self.count.get())
    }

    #[property]
    async fn doubled(&self) -> i64 {
        self.count.get() * 2
    }

    #[setter(doubled)]
    async fn set_doubled(&self, value: i64) -> Result<(), PropertyError> {
        self.count.set(value / 2)
    }

    #[resetter(doubled)]
    async fn reset_doubled(&self) -> Result<(), PropertyError> {
        self.count.reset()
    }

    #[endpoint(get, "status")]
    async fn status(&self) -> String {
        format!("count={}", self.count.get())
    }

    #[on_start]
    async fn started(&self, ctx: ThingCtx) {
        assert_eq!(ctx.name(), "counter");
        self.starts.fetch_add(1, Ordering::SeqCst);
    }

    #[on_stop]
    async fn stopped(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn generated_action_inputs_and_wrappers_work_through_the_facade() {
    let thing = Arc::new(Counter::default());
    let client = TestClient::start(
        ThingServer::builder()
            .thing_arc("counter", thing.clone())
            .build()
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(thing.starts.load(Ordering::SeqCst), 1);
    let response = client.post_json("/counter/add", Some(&json!({}))).await;
    assert_eq!(response.status.as_u16(), 201);
    assert_eq!(response.json()["input"], json!({"step":2}));
    let invocation = client.runtime().invocations().list().pop().unwrap();
    assert_eq!(invocation.wait().await, InvocationStatus::Completed);
    assert_eq!(invocation.output(), Some(json!(4)));
    let counter = client.runtime().thing_ref::<Counter>("counter").unwrap();
    assert_eq!(counter.add(3).await.unwrap(), 7);
    assert_eq!(thing.count.get(), 7);
    assert!(thing.count.set(11).is_err());
    client.stop().await;
    assert_eq!(thing.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn generated_properties_endpoints_and_lifecycle_hooks_are_registered() {
    let client = TestClient::start(
        ThingServer::builder()
            .thing("counter", Counter::default())
            .build()
            .unwrap(),
    )
    .await
    .unwrap();
    let td = client.get("/counter/").await.json();
    assert_eq!(td["properties"]["count"]["minimum"], 0);
    assert_eq!(td["properties"]["count"]["maximum"], 10);
    assert_eq!(client.get("/counter/doubled").await.json(), 4);
    assert_eq!(
        client
            .put_json("/counter/doubled", &json!(12))
            .await
            .status
            .as_u16(),
        201
    );
    assert_eq!(client.get("/counter/count").await.json(), 6);
    assert_eq!(client.get("/counter/status").await.text(), "count=6");
    assert_eq!(
        client
            .post_json("/counter/doubled/reset", None)
            .await
            .status
            .as_u16(),
        200
    );
    assert_eq!(client.get("/counter/count").await.json(), 2);
    client.stop().await;
}

#[derive(Default)]
struct TestDriver;
impl Driver for TestDriver {}

#[derive(Thing)]
struct DeviceThing {
    #[device]
    driver: Device<TestDriver>,
}

#[tokio::test]
async fn derive_without_an_impl_block_initializes_and_manages_devices() {
    let thing = Arc::new(DeviceThing::default());
    let runtime = Runtime::builder()
        .thing_arc("device", thing.clone())
        .build()
        .unwrap();
    runtime.start().await.unwrap();
    assert_eq!(thing.driver.state(), teta_wot::DeviceState::Ready);
    assert_eq!(thing.driver.name(), "device:driver");
    runtime.stop().await;
    assert_eq!(thing.driver.state(), teta_wot::DeviceState::Closed);
}

#[derive(Clone, serde::Serialize, schemars::JsonSchema)]
struct Trip {
    was_emitting: bool,
}

#[derive(Thing)]
struct EventThing {
    #[event(title = "Interlock tripped")]
    tripped: Event<Trip>,
}

#[tokio::test]
async fn derived_events_register_their_schema_and_deliver_to_rust_and_sse() {
    let thing = Arc::new(EventThing::default());
    let client = TestClient::start(
        ThingServer::builder()
            .thing_arc("event", thing.clone())
            .build()
            .unwrap(),
    )
    .await
    .unwrap();
    let td = client.get("/event/").await.json();
    assert_eq!(td["events"]["tripped"]["title"], "Interlock tripped");
    assert_eq!(
        td["events"]["tripped"]["data"]["properties"]["was_emitting"]["type"],
        "boolean"
    );
    let mut rust = thing.tripped.subscribe();
    let mut sse = client.events("/event/tripped").await;
    assert_eq!(sse.status.as_u16(), 200);
    thing.tripped.emit(Trip { was_emitting: true });
    assert!(rust.recv().await.unwrap().was_emitting);
    assert_eq!(sse.next().await, Some(json!({"was_emitting":true})));
    drop(sse);
    client.stop().await;
}
