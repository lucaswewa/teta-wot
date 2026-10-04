# Writing a Thing

This page writes LabThings' tutorial Thing: a light whose brightness can be set, and which can be turned on and off. It is the [`light`](https://github.com/lucaswewa/teta-wot/tree/main/examples/light) example.

```rust,ignore
{{#include ../../../../examples/light/src/main.rs:thing}}
```

Serve it with a builder (or from a [configuration file](running.md)):

```rust,ignore
let server = ThingServer::builder().thing("light", Light::default()).build()?;
server.serve_with(TcpListener::bind(("127.0.0.1", 5000)).await?, shutdown_signal()).await?;
```

Then, while it runs:

- <http://127.0.0.1:5000/light/> is its Thing Description;
- <http://127.0.0.1:5000/docs> lets you try it: `PUT` a brightness, `POST` to `toggle`, `GET` the status.

## Properties

`brightness` and `is_on` are *data properties*: values the Thing keeps, which work like variables.

- **`brightness` is an `i64`, constrained to 0–100** by `ge` and `le`. A client that writes 150 gets LabThings' 422 answer, with pydantic's message: "Input should be less than or equal to 100". The TD says `"minimum": 0, "maximum": 100`.
- **`is_on` is `readonly`.** Clients can't write it (a `PUT` is 405), but the Thing can: `self.is_on.update(…)` or `self.is_on.set(…)`.
- **Every data property has a default,** which the TD shows.

`status` is a *functional property*: a method whose value is computed when it is read. Functional properties are read-only unless they have a `#[setter]`. See [Properties](../properties.md).

## Actions

`toggle` is an action. Clients invoke it with a `POST`, and the server answers `201 Created` with the invocation at once. The action runs in the background, and the invocation's URL follows it (`pending`, `running`, `completed`). An action can take parameters and return a value, both typed: see [Actions](../actions.md).

Other Things (and tests) call it directly, in-process:

```rust,ignore
let light = runtime.thing_ref::<Light>("light").expect("the light");
light.toggle().await?; // an `ActionError`; in an `anyhow` function, `.map_err(ActionError::into_anyhow)?`
```

## Where things go

| Python (LabThings) | Rust (`teta-wot`) |
|---|---|
| `class Light(lt.Thing):` with a docstring | `#[derive(Thing)] pub struct Light` with a doc comment |
| `brightness: int = lt.property(default=100, ge=0, le=100)` | `#[property(default = 100, ge = 0, le = 100)] brightness: Prop<i64>` |
| `self.brightness` | `self.brightness.get()` |
| `self.is_on = True` | `self.is_on.set(true)?` |
| `@lt.action def toggle(self):` | `#[action] async fn toggle(&self)` in `#[thing_impl]` |
| `@lt.property def status(self) -> str:` | `#[property] async fn status(&self) -> String` |

The [migration guide](../migration.md) has the complete table.
