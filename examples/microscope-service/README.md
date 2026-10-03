# microscope-service

**Concept:** a Thing server as a Windows service: the [simulated microscope](../simulated-microscope/README.md) installed, started and stopped by the Service Control Manager, stopping gracefully when the service stops.

The plan called this example `windows-service`, but that name is also a crate on crates.io, which made `cargo run -p windows-service` ambiguous.

**Technologies:**

- the `windows-service` feature, `wot::server::service` (`run_as_service`, `install`, `uninstall`);
- `cli::serve_until` with the service's stop signal;
- `tracing-subscriber` writing to a file;
- PowerShell's `Start-Service` and `Stop-Service`.

## Run it

```
cargo run -p microscope-service
```

Without arguments, it demonstrates the stop path without the Service Control Manager (and without an administrator):

1. it serves the microscope as the service would;
2. it starts a long move;
3. it completes the same shutdown future that the Service Control Manager's *stop* request completes;
4. it checks that the server stopped cleanly, cancelling the move rather than waiting for it.

As a real service, from an administrator console:

```
cargo build -p microscope-service
target\debug\microscope-service.exe install --port 5000
Start-Service teta-wot-microscope
# … use http://127.0.0.1:5000/ …
Stop-Service teta-wot-microscope
target\debug\microscope-service.exe uninstall
```

`install` writes the service's configuration (`microscope-service.json`, with an absolute settings folder) next to the executable, and the service logs to `microscope-service.log` there. `microscope-service console -c FILE` runs the same server in a console.

`service_test.ps1` does the whole cycle and checks it (CI runs it, since GitHub's runners are administrators):

```
powershell -ExecutionPolicy Bypass -File examples\microscope-service\service_test.ps1
```
