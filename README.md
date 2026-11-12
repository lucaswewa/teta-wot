[![CI](https://github.com/lucaswewa/teta-wot/actions/workflows/ci.yml/badge.svg)](https://github.com/lucaswewa/teta-wot/actions/workflows/ci.yml)

# teta-wot

`teta-wot` is a Rust framework for serving laboratory instruments, and the software around them, over HTTP as [W3C Web of Things](https://www.w3.org/WoT/) *Things*. A stage, a camera or an autofocus routine becomes a Thing with properties, actions and events. Clients reach it from a browser, `curl`, Python or any Web of Things consumer.

- **A Thing is a Rust struct.** Its properties and actions are fields and methods, marked with attributes. From those alone, the server builds:
  - the HTTP API;
  - each Thing's [Thing Description](https://www.w3.org/TR/wot-thing-description11/);
  - an OpenAPI document, and interactive docs at `/docs`.
- **Types describe the data.** Properties, action inputs and outputs are Rust types. Their JSON Schemas become the Thing Description's data schemas, and invalid input is refused.
- **Built for hardware:**
  - each Thing is created, started and stopped once;
  - blocking drivers run on their own threads;
  - long actions can be cancelled;
  - an optional global lock keeps actions from overlapping;
  - settings persist across restarts.
- **More of the Web of Things:**
  - events;
  - observation over WebSocket and server-sent events;
  - discovery;
  - a wire profile that follows the W3C WoT Profile.
- **Compatible with [LabThings](https://github.com/labthings/labthings-fastapi).** `teta-wot` reads LabThings' configuration files and command line, and LabThings' Python client works against its servers.
- **Binary data:** Blobs, MJPEG streams, and binary WebSocket streams, with each client getting its own.
- **Your own routes and security:**
  - your own axum routes and layers;
  - TLS with HTTP/2;
  - Basic or bearer credentials, or your own authentication and authorisation, so every affordance knows who is calling.
- **One binary.** A server is a single executable. It can run as a Windows service or under systemd, and stops gracefully with either.

`teta-wot` runs on **Windows** and **Linux**, on x86_64, and CI tests both. It is under the [MIT licence](LICENSE).

## Installation

**The toolchain.** Install [rustup](https://rustup.rs/).

- **On Windows,** choose the MSVC toolchain, and install Visual Studio's C++ build tools when necessary.
- **On Linux,** install a C toolchain first. On Ubuntu: `sudo apt install build-essential pkg-config`.

`teta-wot` needs Rust 1.98 or newer.

**The dependency.** `teta-wot` isn't published to crates.io: depend on a tag of this repository.

```toml
[dependencies]
teta-wot = { git = "https://github.com/lucaswewa/teta-wot", tag = "v0.2.0" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

[Releases](https://github.com/lucaswewa/teta-wot/releases) list the tags.

- **Depend on the `teta-wot` crate alone.** The other crates in `crates/` sit behind it, and may change shape in any release.
- **In code it is `teta_wot`.** `use teta_wot::prelude::*;` brings in what Thing code usually needs.

**Features**, all off by default:

| Feature | What it adds |
|---|---|
| `testing` | An in-process HTTP client and a harness for testing Things |
| `validation` | Validating Thing Descriptions against the W3C JSON Schema |
| `ndarray` | `ndarray` arrays as properties and action inputs and outputs |
| `mdns` | Advertising the server on the local network with DNS-SD |
| `docs-offline` | The interactive docs' assets compiled in, for networks without internet access |
| `tls` | Serving over TLS, with HTTP/2 for clients that negotiate it |
| `windows-service` | Running as a Windows service, and installing one (Windows only) |

**Without Rust,** each release has a Windows package with the simulated microscope's server, ready to run.

## How to use the library

A Thing is a struct and an `impl` block. A server holds Things, each under a name:

```rust
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use tokio::net::TcpListener;

/// A counter that clients can read and increment.
#[derive(Thing)]
pub struct Counter {
    /// How many times `increment` has run.
    #[property(default = 0, readonly)]
    count: Prop<i64>,
}

#[thing_impl]
impl Counter {
    /// Adds one to the count.
    #[action]
    async fn increment(&self) -> Result<(), ActionError> {
        self.count.update(|c| *c += 1)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = ThingServer::builder()
        .thing("counter", Counter::default())
        .build()?;
    let listener = TcpListener::bind(("127.0.0.1", 5000)).await?;
    server.serve_with(listener, shutdown_signal()).await?;
    Ok(())
}
```

- **`#[derive(Thing)]`** makes the struct a Thing. Doc comments become titles and descriptions.
- **`#[property]`** on a `Prop<T>` field makes a property, which clients read and, unless it's `readonly`, write.
- **`#[action]`** methods in a `#[thing_impl]` block are actions, which clients invoke.
- **`shutdown_signal()`** completes on Ctrl-C, or when the system asks the program to stop. The server then stops gracefully.

Run it with `cargo run`, then:

- **the Thing Description** is at <http://127.0.0.1:5000/counter/>;
- **the interactive API docs** are at <http://127.0.0.1:5000/docs>;
- **from `curl`:**

  ```text
  curl http://127.0.0.1:5000/counter/count
  curl -X POST http://127.0.0.1:5000/counter/increment
  ```

  The `POST` answers `201 Created` at once, with the invocation's URL in `Location`. The action runs in the background, and the invocation follows it to `completed`.

**From here:**

- **[The guide](docs/guide/src/SUMMARY.md)** covers writing Things, serving them, and using them. It is an mdBook: `mdbook build docs/guide` writes it to `target/guide/`.
  - **Writing Things:**
    - [actions](docs/guide/src/actions.md);
    - [properties](docs/guide/src/properties.md);
    - [events](docs/guide/src/events.md);
    - [settings](docs/guide/src/settings.md);
    - [concurrency and hardware](docs/guide/src/concurrency.md);
    - [testing](docs/guide/src/testing.md).
  - **Serving:**
    - [configuration files and the command line](docs/guide/src/configuration.md);
    - [wire profiles](docs/guide/src/wire_profiles.md);
    - [discovery](docs/guide/src/discovery.md);
    - [security](docs/guide/src/security.md);
    - [your own routes, layers and TLS](docs/guide/src/composition.md).
  - **Coming from LabThings, or from 0.1:** [Migrating](docs/guide/src/migration.md).
- **[The changelog](CHANGELOG.md)** says what changed in each release.

## Examples

[`examples/`](examples/) has 39 small crates, each teaching one thing with the public API. Each has a README saying what it shows and how to run it, and CI runs every one.

```sh
cargo run -p quickstart-counter              # a short demonstration in-process, then exit
cargo run -p quickstart-counter -- --serve   # serve on http://127.0.0.1:5000 until Ctrl-C
```

Most examples work the same way.

| To learn | Examples |
|---|---|
| A first Thing | [`quickstart-counter`](examples/quickstart-counter), [`light`](examples/light), [`counter-server`](examples/counter-server), [`builder_thing`](examples/builder_thing) |
| Properties | [`property-cells`](examples/property-cells), [`functional-properties`](examples/functional-properties), [`settings`](examples/settings) |
| Actions | [`cancellable-action`](examples/cancellable-action), [`invocation-logs`](examples/invocation-logs), [`dependency-injection`](examples/dependency-injection) |
| Hardware | [`device-actor`](examples/device-actor), [`global-lock`](examples/global-lock), [`graceful-shutdown`](examples/graceful-shutdown) |
| Composing Things | [`thing-slots`](examples/thing-slots), [`config-and-cli`](examples/config-and-cli) |
| Events and observation | [`events`](examples/events), [`observe-websocket`](examples/observe-websocket), [`sse-observe`](examples/sse-observe) |
| Binary data | [`blob-camera`](examples/blob-camera), [`mjpeg-stream`](examples/mjpeg-stream), [`binary-stream`](examples/binary-stream), [`ndarray-property`](examples/ndarray-property) |
| Custom HTTP | [`custom-endpoint`](examples/custom-endpoint), [`path-parameters`](examples/path-parameters), [`custom-routes`](examples/custom-routes), [`api-prefix`](examples/api-prefix), [`http-walkthrough`](examples/http-walkthrough) |
| Security | [`bearer-security`](examples/bearer-security), [`custom-auth`](examples/custom-auth) |
| Thing Descriptions and API docs | [`td_builder`](examples/td_builder), [`schema-to-dataschema`](examples/schema-to-dataschema), [`units-and-semantics`](examples/units-and-semantics), [`openapi-export`](examples/openapi-export), [`docs-offline`](examples/docs-offline) |
| The W3C Web of Things | [`strict-profile`](examples/strict-profile), [`discovery`](examples/discovery), [`node-wot-consumer`](examples/node-wot-consumer) |
| Everything together | [`simulated-microscope`](examples/simulated-microscope), [`microscope-service`](examples/microscope-service) |

**The simulated microscope** is the capstone:

- **`Stage`** moves in x, y and z;
- **`Camera`** renders what it sees through the stage into a live MJPEG preview;
- **`Autofocus`** sweeps the stage's z through the camera's sharpness;
- they are composed with slots, served from a configuration file, and driven from Python.

```sh
cargo run -p simulated-microscope -- -c examples/simulated-microscope/microscope.json
```

Then open <http://127.0.0.1:5000/camera/preview/viewer>, and move the stage from <http://127.0.0.1:5000/docs>. To drive it from Python:

```sh
pip install labthings-fastapi==0.3.0 httpx
python examples/simulated-microscope/demo.py
```
