# Properties

A property is a value clients read, and perhaps write: a position, an exposure time, a temperature. There are two kinds.

## Data properties

A data property is a `Prop<T>` field: a value the Thing keeps.

```rust,ignore
/// The exposure time, in milliseconds.
#[property(default = 20.0, gt = 0, le = 1000)]
exposure: Prop<f64>,
```

- **Clients** read it with `GET /camera/exposure` and write it with `PUT`. The written value is validated first.
- **The Thing** reads it with `self.exposure.get()` (a copy) or `self.exposure.read(|v| …)`, and changes it with `set(value)` or `update(|v| …)`. Its own writes are checked against the constraints too, and publish the change to observers.
- **Other code** watches it with `subscribe()`, a `tokio::sync::watch::Receiver`: the camera's preview thread reads the exposure that way.

Options:

| Option | Effect |
|---|---|
| `default = expr` | The initial value, shown in the TD (required, unless the type has a `Default`) |
| `readonly` | Clients can't write it (`PUT` is 405); the Thing can |
| `ge`, `gt`, `le`, `lt`, `multiple_of` | Numeric constraints: `minimum`, `exclusiveMinimum`, … in the TD |
| `min_length`, `max_length`, `pattern` | String (or list) constraints |
| `allow_inf_nan = false` | Refuse infinities and NaN |
| `unit = "mm"`, `semantic_type = "…"` | The TD's `unit` and `@type` |
| `global_lock = false` | Writes don't take the global lock |
| `title`, `description` | Override the doc comment |

Any type that is `Clone`, serialisable and has a JSON Schema can be a property: numbers, strings, `Option<T>`, enums, structs, vectors, maps, [arrays](arrays.md).

## Functional properties

A functional property is a method, computed when it is read:

```rust,ignore
/// A human-readable status of the light.
#[property]
async fn status(&self) -> String {
    if self.is_on.get() {
        format!("The light is on at {}% brightness.", self.brightness.get())
    } else {
        "The light is off.".to_owned()
    }
}
```

It is read-only unless it has a setter, and can have a resetter:

```rust,ignore
#[setter(status)]
async fn set_status(&self, value: String) -> Result<(), PropertyError> { … }

#[resetter(status)]
async fn reset_status(&self) -> Result<(), PropertyError> { … }
```

A getter or setter that talks to hardware synchronously is marked `blocking` and written as `fn`: it runs on a blocking thread. Failures are `PropertyError`s. A getter's failure is a 500 answer.

## Resetting

`POST /{thing}/{property}/reset` resets a writable property that has a default, or a `#[resetter]`, to its default. Read-only properties can't be reset.

## Observation

Data properties are observable: their changes are published to observers.

- **Over WebSocket** (`/{thing}/ws`, `addPropertyObservation`).
- **As server-sent events:** `GET /camera/exposure` with `Accept: text/event-stream`.
- **All of a Thing's properties at once:** `GET /camera/properties` with the same header.

Their TD says `"observable": true`, with a form for each. Functional properties aren't observable, since nothing publishes their changes.

## Settings

A data property marked `#[setting]` instead of `#[property]` is saved to the Thing's settings file after every change, and loaded when the server starts: see [Settings](settings.md).
