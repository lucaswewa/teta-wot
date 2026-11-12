# Migrating

This page is for code that already exists: Things written for LabThings in Python, and applications built on `teta-wot` 0.1.

## From LabThings in Python

A Thing keeps its shape: the same properties, actions and settings, and the same configuration file. These are the constructs that change spelling:

| LabThings (Python) | `teta-wot` (Rust) | See |
|---|---|---|
| `class Light(lt.Thing):` with a docstring | `#[derive(Thing)] pub struct Light` with a doc comment | [Structure](structure.md) |
| `brightness: int = lt.property(default=100, ge=0, le=100)` | `#[property(default = 100, ge = 0, le = 100)] brightness: Prop<i64>` | [Properties](properties.md) |
| `self.brightness` | `self.brightness.get()` | [Properties](properties.md) |
| `self.is_on = True` | `self.is_on.set(true)?` | [Properties](properties.md) |
| `@lt.property def status(self) -> str:` | `#[property] async fn status(&self) -> String` in `#[thing_impl]` | [Functional properties](properties.md#functional-properties) |
| `@lt.action def toggle(self):` | `#[action] async fn toggle(&self)` in `#[thing_impl]` | [Actions](actions.md) |
| A property saved to the settings file | `#[setting]` in place of `#[property]`. The file's format is LabThings', so both servers can share it | [Settings](settings.md) |
| Other Things, given by the configuration's `thing_slots` | `#[slot]` fields: `Slot<T>`, `OptSlot<T>` or `SlotMap<T>` | [Thing slots](thing_slots.md) |
| `enable_global_lock` | The same key. Actions and properties opt out with `global_lock = false` | [The global lock](concurrency.md#the-global-lock) |
| `NDArray` | `NdArray`, with the `ndarray` feature | [Arrays](arrays.md) |
| `labthings-server -c config.json` | Your application's binary, which hands its command line to `teta_wot::server::cli`. It has the same options and exit codes | [Running a server](tutorial/running.md) |
| The configuration file | The same format. `teta-wot` adds `server_id`, `wire_profile`, `security` and `mdns`, which LabThings ignores | [Configuration](configuration.md) |
| `labthings_fastapi.ThingClient` | Works against `teta-wot` servers as it is. In Rust, Things use each other in-process, through `ThingRef`s | [Using Things](using_things.md) |

Blobs, MJPEG streams, events and custom endpoints have chapters of their own. The default wire profile, `tetathing`, answers with LabThings' status codes and error bodies. [Wire profiles](wire_profiles.md) compares it with the W3C one.

## From `teta-wot` 0.1 to 0.2

Point the dependency at the new tag:

```toml
[dependencies]
teta-wot = { git = "https://github.com/lucaswewa/teta-wot", tag = "v0.2.0" }
```

These are unchanged:

- the minimum Rust version, 1.98;
- configuration files and the command line;
- settings files;
- Thing Description `id`s;
- problem types;
- both wire profiles' answers, except to WebSocket upgrades without credentials (below).

### Code that may not compile

Two structs gained a public field. Code that builds them with a struct literal must add it:

- **`HttpOptions`** gained `auth`. End the literal with `..Default::default()`, or add `auth: Default::default()`:

  ```rust,ignore
  let options = HttpOptions {
      api_prefix: "/api/v1".into(),
      ..HttpOptions::default()
  };
  ```

- **`EndpointLink`** gained `members`. Add `members: Default::default()`, or build the link with `EndpointSpec::link`, which sets it.

### Behaviour to check

- **WebSockets on a server that requires credentials.** An upgrade request without an `Authorization` header used to be refused with `401`.
  - **Now the observation WebSocket and binary streams accept it,** and wait for the credentials in the first message: `{"authorization": "Bearer …"}`, the value the header would have held.
  - **A wrong value, or nothing within 10 seconds,** closes the socket with code 1008.
  - **A client that relied on the `401`** should watch for that close instead.
  - **A request with the header** is still refused before the upgrade (`401` or `403`).
  - See [WebSockets](security.md#websockets).
- **HTTP/2 without TLS.** `serve` and `serve_with` also accept clients that speak HTTP/2 in the clear ("h2c"). HTTP/1.1 clients see no difference.
- **Signals on Linux.** `shutdown_signal` now also completes on `SIGTERM` and `SIGHUP`, so `kill` and systemd stop a server gracefully. In 0.1, `SIGTERM` ended the process at once.
- **The OpenAPI document's version.** Unless `ThingServerBuilder::api_info` sets one, `info.version` is now `teta-wot`'s version, `0.2.0`; it used to be `0.1.0`.

### What's new

Nothing here needs a change, but each item may replace code you wrote yourself:

- path parameters in custom endpoints, such as `"tiles/{level}/{tile}"` ([Structure](structure.md#a-things-type));
- your own axum routes and layers, custom listeners, and TLS with HTTP/2 ([Your own routes, layers and TLS](composition.md));
- binary streams, a WebSocket of binary messages for each client ([Binary streams](blobs.md#binary-streams));
- your own authentication and authorisation, and `Principal::current()` for who is calling ([Security](security.md#your-own-authentication-and-authorisation));
- Linux ([Installing](tutorial/installing.md)).

The [changelog](https://github.com/lucaswewa/teta-wot/blob/main/CHANGELOG.md) lists everything.
