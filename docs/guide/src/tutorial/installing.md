# Installing teta-wot

## The toolchain

`teta-wot` is built with Rust on Windows x86_64.

1. Install [rustup](https://rustup.rs/), with the MSVC toolchain and Visual Studio's C++ build tools when it asks.
2. The repository pins its compiler in `rust-toolchain.toml`, and rustup installs it on first use.
3. Applications that depend on `teta-wot` need that version or a newer one.

## Depending on `wot`

Applications depend on one crate, `wot`, from the Git repository:

```toml
[dependencies]
wot = { git = "https://github.com/lucaswewa/wot", tag = "v0.1.0" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

Pin a tag (or a `rev`) so that builds don't change under you. [Releases](https://github.com/lucaswewa/wot/releases) list the tags. The other crates in the repository (`wot-core`, `wot-http`, …) are behind `wot`, and may change shape in any release.

`use wot::prelude::*;` brings in what Thing code usually needs.

## Features

| Feature | What it adds |
|---|---|
| `testing` | `wot::testing`: an in-process HTTP client and a harness for tests ([Testing](../testing.md)) |
| `validation` | Validating Thing Descriptions against the W3C JSON Schema |
| `ndarray` | `NdArray`, arrays as nested lists ([Arrays](../arrays.md)) |
| `mdns` | Advertising the server with DNS-SD ([Discovery](../discovery.md)) |
| `docs-offline` | Swagger UI and ReDoc compiled into the binary, for networks without internet access |
| `windows-service` | Running as a Windows service ([Windows service](../windows_service.md)) |

## Without Rust

Each [release](https://github.com/lucaswewa/wot/releases) has a Windows package with the simulated microscope's server, ready to run, for trying `teta-wot` out.

## The Python side

LabThings' Python client, `labthings_fastapi.ThingClient`, works against `teta-wot` servers:

```text
pip install labthings-fastapi==0.3.0
```

The repository's conformance project (`conformance/reference`, managed with [uv](https://docs.astral.sh/uv/)) has it, with everything the examples' Python scripts use:

```text
uv run --locked --project conformance/reference python examples/simulated-microscope/demo.py --spawn target/debug/simulated-microscope.exe
```
