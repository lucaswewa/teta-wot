# Changelog

All notable changes to `teta-wot` are recorded here, in the format of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-12

Linux support, path parameters, your own routes and layers, TLS and HTTP/2, binary streams, and pluggable authentication. Upgrading from 0.1 needs code changes only where `HttpOptions` or `EndpointLink` is built with a struct literal. WebSocket clients of servers that require credentials are refused differently. The guide's [*Migrating*](https://github.com/lucaswewa/teta-wot/blob/v0.2.0/docs/guide/src/migration.md) page has the details.

### Added

- **Pluggable authentication and authorisation.**
  - **An `Authenticator`** says who sent a request, as a `Principal` (an id and attributes), or refuses it with a `Rejection`: `401` with a challenge, or `403`.
  - **An `Authorizer`** says whether a principal may make a request, from its `Access`: the Thing, the affordance, the `Operation`, and whether it changes state.
  - **Setting them up:** `ThingServerBuilder::authenticator` and `authorizer`, or `HttpOptions::auth`. Either may be left out. `Security` (Basic or bearer) is an authenticator too, so existing servers are unchanged. The TDs describe the authenticator's `security_scheme`.
  - **Every request is checked:** properties, actions, observation, streams, Blobs, custom endpoints and merged routes. TDs, discovery and the docs stay public.
  - **Who is calling.** `Principal::current()` gives the caller to the code a request runs: getters, setters, actions (also as `ActionCtx::principal`), endpoints, merged routes and binary-stream producers (also as `Subscriber::principal`).
  - **WebSockets authenticate with their first message,** `{"authorization": "…"}`, when their upgrade request has no `Authorization` header, as browsers can't add one. A failure, or no message within 10 seconds, closes the socket with 1008. This covers the observation WebSocket and binary streams. A custom endpoint marked with `EndpointSpec::websocket` can do the same with `teta_wot::http::FirstMessageAuth`.
  - **A new example: `custom-auth`.** API keys for operators and viewers, and a logbook that signs its entries.
- **Binary streams.** `#[binary_stream]` methods, or `Endpoint::binary_stream`, make a WebSocket of binary messages for each client.
  - **The producer** runs for each connection with a `Subscriber<P>`.
  - **Parameters.** The client sends its parameters as JSON, which are deserialised into `P`; bad ones are answered with `{"error": …}`.
  - **Sending.** `send` never waits. A message the client hasn't taken is replaced by the next and counted as dropped, so a slow client gets the newest message and holds up no one.
  - **The end.** Streams end when the client closes or the server shuts down.
  - **The TD** links each stream, with its `subprotocol` (`websocket`), media type and parameter schema.
  - **A new example: `binary-stream`.** It streams 1 MB at 60 Hz to four clients: about 63 MB/s each, with nothing dropped, on a Windows laptop.
- **`EndpointSpec::link_member`** adds a member to an endpoint's TD link.
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

- **`HttpOptions` has a new public field, `auth`.** Code that builds `HttpOptions` with a struct literal must add `auth: Default::default()`, or end with `..Default::default()`.
- **WebSocket upgrades without credentials,** on a server that requires them, are no longer refused with `401`. The observation WebSocket and binary streams accept them, then wait for the credentials in the first message.
- **`EndpointLink` has a new public field, `members`,** for the link's other members. Code that builds an `EndpointLink` with a struct literal must add `members: Default::default()`; `EndpointSpec::link` sets it.
- **HTTP/2 everywhere.** `axum` is built with HTTP/2, so `serve` and `serve_with` also accept HTTP/2 without TLS ("h2c"), from clients that use it. The `h2` crate is now a dependency.
- **The licence is stated:** the workspace's crates declare `license = "MIT"`, matching `LICENSE`. The guide no longer says there is no licence.
- **`rust-toolchain.toml` no longer pins a target,** so each platform builds for itself.
- **`microscope-service`,** a Windows service, prints that it is Windows-only and exits on other systems, rather than failing to build.
- **The OpenAPI document's default version is `teta-wot`'s** (here `0.2.0`), where 0.1 always said `0.1.0`. `ThingServerBuilder::api_info` sets your API's own.
- **Documentation:**
  - the guide covers Linux (installing, stopping a server), and uses the crate's current name, `teta-wot` (`teta_wot::` in code), and its repository's URL;
  - a new page, *Migrating*, maps LabThings' Python constructs to `teta-wot`'s, and says how to move from 0.1 to 0.2;
  - the features table lists `tls`;
  - `docs/releasing.md` no longer links to an ADR that isn't in the repository.

### Fixed

- **`RouteError::ForeignEndpoint`'s message** named the crate's old name, `wot_http`. It now names `teta_wot::http::Endpoint`.
- **The guide's links to its migration guide** led nowhere: the page didn't exist.
- **The Windows package's licences.**
  - It carries `THIRD_PARTY_LICENSES.txt` again: the licence texts of the crates compiled into its binaries.
  - It now carries `THIRD_PARTY_NOTICES.md`, for the files copied into the sources: Swagger UI, ReDoc and the W3C TD JSON Schema.
  - Its README acknowledges the Independent JPEG Group's work, which the JPEG encoder is based on, as the encoder's licence asks.

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

[0.2.0]: https://github.com/lucaswewa/teta-wot/releases/tag/v0.2.0
[0.1.0]: https://github.com/lucaswewa/teta-wot/releases/tag/v0.1.0
