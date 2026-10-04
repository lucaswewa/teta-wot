# Running as a Windows service

An instrument server usually runs unattended: started with Windows, stopped cleanly when Windows shuts down. On Windows that is a *service*. With the `windows-service` feature, a `teta-wot` server runs as one, and installs itself.

## Running as a service

`teta_wot::server::service::run_as_service` connects the process to the Service Control Manager, and runs the server with the service's *stop* request as its shutdown signal:

```rust,ignore
{{#include ../../../examples/microscope-service/src/main.rs:run}}
```

- **Starting.** The service reports *running* once the server is serving.
- **Stopping.** On *stop* (or when Windows shuts down), it stops as a console server does on Ctrl-C:
  1. invocations are cancelled;
  2. requests finish, up to the grace period;
  3. the Things stop, and their settings are saved and their devices closed.

  Meanwhile it reports *stop pending*, with a wait hint (`STOP_HINT`) that should cover the grace period.
- **The exit code.** When the server returns, the service reports *stopped* with its exit code.
- **Outside a service.** If the process wasn't started as a service, `run_as_service` returns `ServiceError::NotAService` at once, so one binary can run in a console too.

## Installing

`service::install` registers the executable, with its arguments, and `service::uninstall` removes it. Both need an administrator:

```text
microscope-service install --port 5000
Start-Service teta-wot-microscope
Stop-Service teta-wot-microscope
microscope-service uninstall
```

A service starts in `C:\Windows\System32`, as the LocalSystem account, with no console.

- **Every path must be absolute:** configuration, settings folder, log.
- **Logging goes to a file:** set up a `tracing` subscriber that writes to one before serving.

The example's `install` writes its configuration and log next to the executable.

## The example

The [`microscope-service`](https://github.com/lucaswewa/teta-wot/tree/main/examples/microscope-service) example is the simulated microscope as a service, with `install`, `uninstall`, `run` (what the Service Control Manager starts) and `console` commands. Run without arguments, it demonstrates the stop path without the Service Control Manager.

`service_test.ps1` then checks the real thing, from an administrator console:

1. it installs the service, starts it, and uses it;
2. it stops it during a long action;
3. it checks that the stop was graceful;
4. it removes it.
