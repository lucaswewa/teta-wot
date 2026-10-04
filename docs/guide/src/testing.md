# Testing

With the `testing` feature, `teta_wot::testing` tests Things without a network.

## An in-process client

`TestClient` sends HTTP requests straight to a server's router, in the same process: no port, no sockets, the exact answers a client would get.

```rust,ignore
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

#[tokio::test]
async fn toggling_turns_the_light_on() {
    let server = ThingServer::builder().thing("light", Light::default()).build().unwrap();
    let client = TestClient::start(server).await.unwrap();

    client.post_json("/light/toggle", None).await;
    // … wait for the invocation (see below), then:
    assert_eq!(client.get("/light/is_on").await.json(), true);

    let refused = client.put_json("/light/brightness", &serde_json::json!(150)).await;
    assert_eq!(refused.status, 422);
    client.stop().await;
}
```

- `get`, `put_json`, `post_json` and `delete` send requests; `request` sends anything.
- `events(path)` opens a server-sent events stream; `next().await` reads its data, and `next_message()` its name and ID too.
- `websocket(path)` opens a WebSocket (served on a loopback port for as long as it's open).
- `stream(path)` gets a long response, such as MJPEG, without reading it all.
- `with_host` and `with_header` set the `Host` and any header (such as credentials) for every request.
- `runtime()` reaches the Things themselves, for in-process calls.

## A harness for one Thing

`Harness` starts one Thing, with stand-ins for its slots and a temporary settings folder:

```rust,ignore
let harness = Harness::builder("autofocus", Autofocus::default())
    .thing("stage", Stage::default())
    .thing("camera", FakeCamera::default())
    .start()
    .await?;
let focus = harness.thing().run(1000, 21).await?;     // in-process, typed
let http = harness.client().get("/autofocus/").await; // or over HTTP
harness.stop().await;
```

## Actions without a server

`ActionCtx::fake()` and `teta_wot::testing::action_ctx("thing")` make an invocation context for calling action code directly, and `in_invocation(&ctx, future)` runs code as if in an invocation (for its logs and cancellation).

## Waiting for invocations

A `POST` answers before the action finishes. Poll the invocation's `href`, or call the action in-process, where the call returns its output:

```rust,ignore
let light = client.runtime().thing_ref::<Light>("light").expect("the light");
light.toggle().await?;
```
