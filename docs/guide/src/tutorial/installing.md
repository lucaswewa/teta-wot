# Installing teta-wot

## The toolchain

`teta-wot` is built with Rust, on Windows or Linux, on x86_64.

1. Install [rustup](https://rustup.rs/).
   - **On Windows,** choose the MSVC toolchain, and install Visual Studio's C++ build tools when it asks.
   - **On Linux,** install a C toolchain first. On Ubuntu: `sudo apt install build-essential pkg-config`.
2. The repository pins its compiler in `rust-toolchain.toml`, and rustup installs it on first use.
3. Applications that depend on `teta-wot` need that version or a newer one.

## Depending on `teta-wot`

Applications depend on one crate, `teta-wot`, from the Git repository:

```toml
[dependencies]
teta-wot = { git = "https://github.com/lucaswewa/teta-wot", tag = "v0.2.0" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

Pin a tag (or a `rev`) so that builds don't change under you. [Releases](https://github.com/lucaswewa/teta-wot/releases) list the tags. The other crates in the repository (`teta-wot-core`, `teta-wot-http`, …) are behind `teta-wot`, and may change shape in any release.

In Rust code the crate is `teta_wot`: `use teta_wot::prelude::*;` brings in what Thing code usually needs.

## Features

| Feature | What it adds |
|---|---|
| `testing` | `teta_wot::testing`: an in-process HTTP client and a harness for tests ([Testing](../testing.md)) |
| `validation` | Validating Thing Descriptions against the W3C JSON Schema |
| `ndarray` | `NdArray`, arrays as nested lists ([Arrays](../arrays.md)) |
| `mdns` | Advertising the server with DNS-SD ([Discovery](../discovery.md)) |
| `docs-offline` | Swagger UI and ReDoc compiled into the binary, for networks without internet access |
| `windows-service` | Running as a Windows service ([Windows service](../windows_service.md)). It has no effect on other systems |
| `tls` | Serving over TLS, with HTTP/2 for clients that negotiate it ([TLS and HTTP/2](../composition.md#tls-and-http2)) |

## Without Rust

Each [release](https://github.com/lucaswewa/teta-wot/releases) has a Windows package with the simulated microscope's server, ready to run, for trying `teta-wot` out.

## The Python side

LabThings' Python client, `labthings_fastapi.ThingClient`, works against `teta-wot` servers:

```text
pip install labthings-fastapi==0.3.0 httpx
```

The simulated microscope's `demo.py` uses both. With `--spawn`, it starts the server itself, on a free port:

```text
cargo build -p simulated-microscope
python examples/simulated-microscope/demo.py --spawn target/debug/simulated-microscope.exe
```

On Linux, the server is `target/debug/simulated-microscope`.
