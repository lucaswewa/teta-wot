# Using Things

## From anything that speaks HTTP

| Operation | Request |
|---|---|
| Read a property | `GET /stage/position` |
| Write a property | `PUT /camera/exposure` with a JSON body (`30.0`) |
| Invoke an action | `POST /stage/move_to` with its input (`{"x": 400, "y": 200, "z": 250}`); `201` and the invocation |
| Follow an invocation | `GET` its `href` (also in `Location`) |
| Cancel it | `DELETE` its `href` |
| Read every property | `GET /stage/properties` |
| The Thing Description | `GET /stage/` |

Every URL comes from the Thing Description's forms, so a client that reads the TD needs none of this table. The OpenAPI document (`/openapi.json`) describes the same routes for client generators.

## Observing

- **Server-sent events**, which browsers read with `EventSource`:

  ```javascript
  const source = new EventSource("/stage/position");   // Accept: text/event-stream
  source.onmessage = (event) => console.log(JSON.parse(event.data));
  ```

  `GET /stage/arrived` streams an event. `GET /stage/properties` and `/stage/events` stream everything, each event named after its affordance.
- **WebSocket,** `ws://127.0.0.1:5000/stage/ws`:

  ```json
  {"messageType": "addPropertyObservation", "data": {"position": true}}
  ```

  answered by `{"messageType": "propertyStatus", "data": {"position": {…}}}` on every change, and `addActionObservation` and `addEventSubscription` likewise. The [`observe-websocket`](https://github.com/lucaswewa/teta-wot/tree/main/examples/observe-websocket) example has a browser page and a Python script.

## From Rust, in-process

Other Things, and tests, use Things directly, without HTTP:

```rust,ignore
let stage = runtime.thing_ref::<Stage>("stage").expect("configured");
stage.move_to(0, 0, 250).await?;             // an action, typed
let here = ThingRef::thing(&stage).now();    // the Thing itself
```

Things get each other through [slots](thing_slots.md).

## From other Web of Things software

Consumers that follow the W3C specifications read the Thing Description and use its forms. For them, serve the [`wot` wire profile](wire_profiles.md), which follows the W3C WoT Profile. [Eclipse Thingweb node-wot](https://github.com/eclipse-thingweb/node-wot) can:

- read, write, invoke (synchronous actions) and observe;
- observe in the `tetathing` profile only, because it listens for unnamed server-sent events.

The [`node-wot-consumer`](https://github.com/lucaswewa/teta-wot/tree/main/examples/node-wot-consumer) example shows what works in each profile.
