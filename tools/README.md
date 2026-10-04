# Tools

Programs for maintaining and releasing `teta-wot`, not for applications.

| Tool | What it does |
|---|---|
| [`soak/`](soak/src/main.rs) | The soak test. It drives the simulated microscope, running as its own process, with every kind of client for a given time: properties, moves (some cancelled), autofocus runs, Blob captures, MJPEG viewers, server-sent events and a WebSocket. It records requests, errors, latency, the server's memory and handles, and the invocations kept, each minute, and fails on any error, an ended stream, or memory that keeps growing after the warm-up. CI runs it for 3 minutes; releases, for 24 hours ([`docs/releasing.md`](../docs/releasing.md)). |
| [`licenses.py`](licenses.py) | Gathers the licence texts of every crate compiled into the release binaries, from the dependency graph, for the release package ([ADR-0062](../docs/adr/0062-release-process.md)). |
| [`release/README.txt`](release/README.txt) | The release package's README. |

```
cargo build --release -p simulated-microscope -p soak
target\release\soak --server target\release\simulated-microscope.exe --duration 10m --report soak.csv
python tools\licenses.py simulated-microscope microscope-service > THIRD_PARTY_LICENSES.txt
```
