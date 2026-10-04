# Changelog

All notable changes to `teta-wot` are recorded here, in the format of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[0.1.0]: https://github.com/lucaswewa/wot/releases/tag/v0.1.0
