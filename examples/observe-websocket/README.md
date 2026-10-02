# observe-websocket

**Concept:** observing a Thing over its WebSocket, `/{thing}/ws`: a property's new values and an action's status changes are pushed to the client as they happen, instead of polled.

**Technologies:** the WebSocket route of the HTTP binding; a static page served by a custom endpoint, using the browser's `WebSocket`; a Python script using `websockets` and `httpx` (from the conformance project, run with `uv`); `wot::testing::TestClient::websocket` for the in-process demo.

## Run it

```
cargo run -p observe-websocket                     # an in-process demo, then exit
cargo run -p observe-websocket -- --serve          # serve on http://127.0.0.1:5000 until Ctrl-C
```

While it serves, open <http://127.0.0.1:5000/oven/monitor.html>, and press **Start**. Or, in another terminal:
