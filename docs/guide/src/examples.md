# Examples

The repository's [`examples/`](https://github.com/lucaswewa/teta-wot/tree/main/examples) folder has 35 small crates, each teaching one concept with the public API only. Each has a README saying what it shows, how to run it, and what to look for. CI runs every one.

Run any of them with `cargo run -p <name>`: most run a short demonstration in-process and exit, and serve with `--serve`.

| To learn | Start with |
|---|---|
| A first Thing | [`quickstart-counter`](https://github.com/lucaswewa/teta-wot/tree/main/examples/quickstart-counter), [`light`](https://github.com/lucaswewa/teta-wot/tree/main/examples/light) |
| Properties | [`property-cells`](https://github.com/lucaswewa/teta-wot/tree/main/examples/property-cells), [`functional-properties`](https://github.com/lucaswewa/teta-wot/tree/main/examples/functional-properties) |
| Actions and cancellation | [`cancellable-action`](https://github.com/lucaswewa/teta-wot/tree/main/examples/cancellable-action), [`invocation-logs`](https://github.com/lucaswewa/teta-wot/tree/main/examples/invocation-logs) |
| Hardware drivers | [`device-actor`](https://github.com/lucaswewa/teta-wot/tree/main/examples/device-actor), [`global-lock`](https://github.com/lucaswewa/teta-wot/tree/main/examples/global-lock) |
| Composing Things | [`thing-slots`](https://github.com/lucaswewa/teta-wot/tree/main/examples/thing-slots), [`dependency-injection`](https://github.com/lucaswewa/teta-wot/tree/main/examples/dependency-injection), [`settings`](https://github.com/lucaswewa/teta-wot/tree/main/examples/settings) |
| Configuration files and the command line | [`config-and-cli`](https://github.com/lucaswewa/teta-wot/tree/main/examples/config-and-cli) |
| Observation and events | [`observe-websocket`](https://github.com/lucaswewa/teta-wot/tree/main/examples/observe-websocket), [`sse-observe`](https://github.com/lucaswewa/teta-wot/tree/main/examples/sse-observe), [`events`](https://github.com/lucaswewa/teta-wot/tree/main/examples/events) |
| Binary data | [`blob-camera`](https://github.com/lucaswewa/teta-wot/tree/main/examples/blob-camera), [`mjpeg-stream`](https://github.com/lucaswewa/teta-wot/tree/main/examples/mjpeg-stream), [`ndarray-property`](https://github.com/lucaswewa/teta-wot/tree/main/examples/ndarray-property) |
| API documentation | [`openapi-export`](https://github.com/lucaswewa/teta-wot/tree/main/examples/openapi-export), [`docs-offline`](https://github.com/lucaswewa/teta-wot/tree/main/examples/docs-offline) |
| The W3C Web of Things | [`strict-profile`](https://github.com/lucaswewa/teta-wot/tree/main/examples/strict-profile), [`discovery`](https://github.com/lucaswewa/teta-wot/tree/main/examples/discovery), [`node-wot-consumer`](https://github.com/lucaswewa/teta-wot/tree/main/examples/node-wot-consumer), [`bearer-security`](https://github.com/lucaswewa/teta-wot/tree/main/examples/bearer-security) |
| Deployment | [`graceful-shutdown`](https://github.com/lucaswewa/teta-wot/tree/main/examples/graceful-shutdown), [`microscope-service`](https://github.com/lucaswewa/teta-wot/tree/main/examples/microscope-service) |
| Everything together | [`simulated-microscope`](https://github.com/lucaswewa/teta-wot/tree/main/examples/simulated-microscope) |

The [catalogue](https://github.com/lucaswewa/wot/blob/main/examples/README.md) lists all of them.

## The simulated microscope

The capstone composes three Things with slots:

- **`Stage`** moves in x, y and z at a speed that is a setting, with cancellable moves and an `arrived` event.
- **`Camera`** renders what it sees through the stage into an MJPEG preview, blurred away from the focal plane. It captures JPEG Blobs, measures sharpness, and saves its exposure as a setting.
- **`Autofocus`** sweeps the stage's z through the camera's sharpness, returns the focus curve as an array, and emits a `focused` event.

```text
cargo run -p simulated-microscope -- -c examples/simulated-microscope/microscope.json
uv run --locked --project conformance/reference python examples/simulated-microscope/demo.py
```

