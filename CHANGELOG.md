# Changelog

All notable changes to `teta-wot` are recorded here, in the format of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Your own routes and layers.**
  - `ThingServerBuilder::merge` adds an application's axum routes in front of the Things' routes. They get the server's credentials, CORS, layers and graceful shutdown.
  - `ThingServerBuilder::layer` adds a layer around every route.
  - `teta_wot_http::router_with` does the same without the server.
  - A new example: `custom-routes`.
- **`ShutdownToken`.** An extractor, also returned by `shutdown_token()`, that completes when the server starts shutting down. Merged handlers that hold connections open, such as WebSockets, can end them rather than delay the shutdown.
- **Custom listeners.** `ThingServer::serve_with_listener` serves on any `axum::serve::Listener`.
- **TLS and HTTP/2** (feature `tls`).
  - `teta_wot::server::tls` provides `server_config`, which takes a PEM certificate and key, and `TlsListener`, which completes TLS handshakes in parallel.
  - It uses rustls with the `ring` provider.
  - HTTP/2 is served to clients that negotiate it by ALPN.
  - A new guide chapter: *Your own routes, layers and TLS*.

- **Path parameters in custom endpoints.**
  - An endpoint's path can have parameters, as whole segments: `#[endpoint(get, "tiles/{level}/{tile}")]`. Its last segment can be a wildcard, `{*rest}`, for the rest of the path.
  - The new `PathParams` extractor (`teta_wot::http::PathParams`) gives the values, percent-decoded. A value that isn't valid UTF-8 once decoded is a `400`.
  - The most specific route answers: a literal segment beats a parameter, which beats a wildcard. Endpoints whose paths differ only in parameter names fail the server's build with `RouteError::EndpointConflict`.
  - A malformed parameter fails the runtime's build. Examples: `{name}_suffix`, which isn't a whole segment; a wildcard that isn't last; a name used twice.
  - OpenAPI lists the parameters (`in: path`).
  - A new example: `path-parameters`.
- **Linux support.** `teta-wot` builds, tests and runs on Linux x86_64. CI runs every job on Ubuntu 24.04 as well as Windows.
- **`shutdown_signal` handles Unix signals.** On Linux it completes on `SIGINT` (Ctrl-C), `SIGTERM` (`kill`, or systemd stopping a service) and `SIGHUP` (the terminal closing), so a server stops gracefully in each case. It used to handle only Ctrl-C there, and a `SIGTERM` ended the process at once.
- **The soak test runs on Linux.** It reads the server's memory and open file descriptors from `/proc`, and `--server` defaults to the platform's executable name.

### Changed

- **HTTP/2 everywhere.** `axum` is built with HTTP/2, so `serve` and `serve_with` also accept HTTP/2 without TLS ("h2c"), from clients that use it. The `h2` crate is now a dependency.
- **The licence is stated:** the workspace's crates declare `license = "MIT"`, matching `LICENSE`. The guide no longer says there is no licence.
- **`rust-toolchain.toml` no longer pins a target,** so each platform builds for itself.
- **`microscope-service`,** a Windows service, prints that it is Windows-only and exits on other systems, rather than failing to build.
- **Documentation:** the guide covers Linux (installing, stopping a server), and uses the crate's current name, `teta-wot` (`teta_wot::` in code), and its repository's URL. `docs/releasing.md` no longer links to an ADR that isn't in the repository.

## [0.1.0] - 2026-10-03

The first release. Everything below is new.

### Added

- **Thing Descriptions** (`teta-wot::td`): the W3C TD 1.1 model, with builders, a conversion from Rust types (through schemars) to TD DataSchemas, and validation against the W3C JSON Schema. TDs match LabThings-FastAPI v0.3.0's, apart from the documented deltas.
- **The runtime:**
  - Things defined with `#[derive(Thing)]` and `#[thing_impl]`, or with the builder API;
  - data and functional properties with pydantic-compatible validation, and actions with cancellation, invocations, logs and the global lock;
  - device actors for synchronous drivers;
  - slots and interfaces, settings, services and the server handle;
  - events.
- **The HTTP binding and server:**
  - routes, answers and errors;
  - configuration files and command line, and the fallback server;
  - graceful shutdown on Windows' console events.
- **Observation:** WebSocket with event subscriptions; server-sent events; both described in every TD.
- **Binary data:** Blobs as action inputs and outputs, and MJPEG streams with a viewer page.
- **OpenAPI 3.1:** an OpenAPI 3.1 document.
- **The W3C WoT Profile:** the `wot` wire profile (HTTP Basic and SSE Profiles), top-level resources, invocation forms, discovery (`/.well-known/wot`, a TD Directory, DNS-SD with `mdns`), and optional HTTP Basic or bearer credentials.
- **Arrays:** `NdArray`, `ndarray` arrays as properties and action inputs and outputs, carried as nested lists as LabThings' `NDArray` (feature `ndarray`).
- **Windows services:** running a server as a Windows service and installing one (feature `windows-service`).
- **Examples:** 35 of them, from a hand-built TD to the simulated microscope, each run by CI.
- **The guide:** an mdBook guide.
- **Hardening:** fuzzing of validation, the WebSocket parser and request bodies; a shutdown-under-load test; and the soak tool (`tools/soak`).

[0.1.0]: https://github.com/lucaswewa/teta-wot/releases/tag/v0.1.0
