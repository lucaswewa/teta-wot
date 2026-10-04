# Events

An event is a notification a Thing sends when something happens: the stage arrived, an image was captured, an interlock tripped, as the Web of Things describes them.

```rust,ignore
/// The stage arrived where it was sent.
#[event]
arrived: Event<Position>,
```

The Thing emits it with `self.arrived.emit(position)`. That never waits, and works from async code and from any thread, including a driver's. An `Event<T>` field can be cloned and given to a thread (the [`events`](https://github.com/lucaswewa/teta-wot/tree/main/examples/events) example does).

`T`'s JSON Schema is the TD's `data` for the event; `Event<()>` carries no data.

## Subscribing

- **Server-sent events:** `GET /stage/arrived` streams each event's data as JSON. `curl -N http://127.0.0.1:5000/stage/arrived` shows them.
- **All of a Thing's events at once:** `GET /stage/events`, each named after its event.
- ** WebSocket:** `{"messageType": "addEventSubscription", "data": {"arrived": {}}}` on `/stage/ws`. Each event arrives as `{"messageType": "event", "data": {"arrived": {"data": …, "timestamp": "…"}}}`.
- **In Rust:** `event.subscribe()` receives them in-process.

The TD describes each event, with a `subscribeevent` form for each transport.

## Events or observed properties?

- **A property** has a value at any time: observing it gives its changes, and a client that connects late reads the current value.
- **An event** happens: a client that wasn't subscribed misses it. It suits things that are over when they are reported, and that no property holds.
