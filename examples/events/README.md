# events

**Concept:** events. A laser's safety interlock trips on its device thread, and the driver emits an event straight from that synchronous code. Clients receive it over server-sent events and over the WebSocket, and Rust code can subscribe too.

**Technologies:** `#[event]` on an `Event<T>` field; a `Device<D>` driver; the SSE route `GET /{thing}/{event}` and the WebSocket's `addEventSubscription`; `Event::subscribe` for Rust; `curl`.

## Run it

```
cargo run -p events                      # an in-process demo, then exit
cargo run -p events -- --serve           # serve on http://127.0.0.1:5000 until Ctrl-C
```

While it serves, watch the event in one terminal (on Windows, write `curl.exe`):

```
curl -N http://127.0.0.1:5000/laser/tripped
```

and trip the interlock in another:

```
curl -X POST http://127.0.0.1:5000/laser/start_emission
curl -X POST http://127.0.0.1:5000/laser/open_door
```

The first terminal prints `data: {"input":"enclosure door","was_emitting":true}`.
