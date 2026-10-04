# Structure

- **Things.** Each is a Rust value of a type marked `#[derive(Thing)]`, served under a name: `/{name}/` is its Thing Description, and its affordances are below it. There is one instance of each Thing, created once, started once and stopped once.
- **Affordances:**
  - [properties](properties.md), values clients read and write;
  - [actions](actions.md), operations clients invoke;
  - [events](events.md), notifications clients subscribe to.

  They are declared with attributes, and described in the Thing Description from their types and doc comments.
- **The runtime** holds the Things, and everything they share:
  - the message broker that carries changes and events to observers;
  - the invocations of actions;
  - the global lock;
  - the settings files;
  - services.
- **The server** serves the runtime over HTTP, WebSockets and server-sent events, and runs its lifecycle (start, serve, shut down gracefully).

## A Thing's type

```rust,ignore
{{#include ../../../examples/simulated-microscope/src/lib.rs:stage}}
```

Fields marked with an attribute are the Thing's affordances and connections:

| Attribute | Field | What it is |
|---|---|---|
| `#[property]` | `Prop<T>` | A data property |
| `#[setting]` | `Prop<T>` | A data property saved to the settings file |
| `#[event]` | `Event<T>` | An event |
| `#[stream]` | `MjpegStream` | An MJPEG stream |
| `#[slot]` | `Slot<T>`, `OptSlot<T>`, `SlotMap<T>` | Other Things it uses |
| `#[device]` | `Device<D>` | A driver on a thread of its own |

Other fields are private state, initialised with `#[thing(init = …)]` or their `Default`.

Methods go in a `#[thing_impl]` block: `#[action]`, functional `#[property]` getters (with `#[setter]` and `#[resetter]`), custom `#[endpoint]`s, and the lifecycle hooks `#[on_start]` and `#[on_stop]`.

The macros are a front end: `ThingDefinition` builds the same thing by hand (the [`builder-thing`](https://github.com/lucaswewa/teta-wot/tree/main/examples/builder-thing) example), for generated or dynamic Things.

## Lifecycle

1. **Create.** The server builds every Thing from its configuration (`kwargs`), fills its slots, and opens its devices.
2. **Start.** Things start in order: a Thing starts after the Things in its slots. Each Thing's settings are loaded, then its `#[on_start]` hook runs. If one fails, the Things already started are stopped, and the server reports which Thing failed (or serves its fallback page).
3. **Serve,** until the shutdown signal.
4. **Shut down:**
   1. connections are refused and requests in progress finish;
   2. unfinished invocations are cancelled;
   3. the server waits up to the grace period (5 s by default) for both;
   4. the Things stop in reverse order (`#[on_stop]`), and their devices close.

## Crates

Applications use the `teta-wot` crate. Behind it:

| Crate | Contents |
|---|---|
| `wot-td` | The Thing Description model, JSON Schema to DataSchema conversion, validation |
| `wot-core` | The runtime: Things, properties, actions, invocations, events, slots, settings, devices, Blobs, streams, arrays |
| `wot-macros` | `#[derive(Thing)]`, `#[thing_impl]`, `#[wot::interface]` |
| `wot-http` | The HTTP binding: routes, answers, observation, OpenAPI, discovery, security |
| `wot-server` | The server: configuration, registry, command line, lifecycle, Windows service, test client |
