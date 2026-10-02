# sse-observe

**Concept:** observing a property in the browser with `EventSource`. A thermometer's reading is pushed as server-sent events, and the page finds the stream in the Thing Description, as any TD consumer would.

**Technologies:** the SSE form of an observable property (`"subprotocol": "sse"`); `GET` with `Accept: text/event-stream`; the browser's `EventSource`; a background task started by `#[on_start]` and stopped by `#[on_stop]`; `wot::testing::TestClient::events` for the in-process demo.

## Run it

```
cargo run -p sse-observe                     # an in-process demo, then exit
cargo run -p sse-observe -- --serve          # serve on http://127.0.0.1:5000 until Ctrl-C
```

While it serves, open <http://127.0.0.1:5000/thermometer/index.html>. Or watch the raw stream (on Windows, write `curl.exe`):

```
curl -N -H "Accept: text/event-stream" http://127.0.0.1:5000/thermometer/temperature
```
