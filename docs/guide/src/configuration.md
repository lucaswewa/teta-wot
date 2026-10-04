# Configuration and the command line

A server is configured by a configuration file, or in code with `ThingServer::builder()`. The [tutorial](tutorial/running.md) runs one.

## The file

```json
{
    "things": {
        "stage": "microscope.stage:Stage",
        "camera": {"class": "microscope.camera:Camera", "kwargs": {}, "thing_slots": {"stage": "stage"}}
    },
    "settings_folder": "./settings",
    "api_prefix": "",
    "enable_global_lock": false,
    "global_lock_log_level": "INFO",
    "application_config": {"lab": "B12"}
}
```

**Configuration keys**:

| Key | Meaning |
|---|---|
| `things` | Each Thing's name and type: a type name, or `{"class" (or "cls"), "args", "kwargs", "thing_slots"}` |
| `settings_folder` | Where settings files go (`./settings`) |
| `api_prefix` | A prefix for every route, such as `/api/v1` |
| `enable_global_lock` | Whether actions and writes take the global lock |
| `global_lock_log_level` | The level of the "Global lock was busy" message |
| `application_config` | Anything, for Things to read (`Server::application_config`) |

`kwargs` become the Thing's typed configuration: `#[thing(config = StageConfig)]`, where `StageConfig` is a `serde::Deserialize` struct. Unknown keys are logged and ignored.

**`teta-wot`'s keys**:

| Key | Meaning |
|---|---|
| `server_id` | Identifies the server in TD `id`s (the computer's name by default) |
| `wire_profile` | `"tetathing"` (the default) or `"wot"` ([Wire profiles](wire_profiles.md)) |
| `security` | `{"scheme": "basic", "username": "…", "password_env": "VAR"}` or `{"scheme": "bearer", "token_env": "VAR"}` ([Security](security.md)) |
| `mdns` | `true` to advertise with DNS-SD ([Discovery](discovery.md)) |

## In code

```rust,ignore
let server = ThingServer::builder()
    .thing("stage", Stage::default())
    .thing("camera", Camera::default())
    .settings_folder("C:/lab/settings")
    .api_prefix("/api/v1")
    .global_lock(true)
    .wire_profile(WireProfile::Wot)
    .shutdown_grace(Duration::from_secs(10))
    .build()?;
server.serve(([0, 0, 0, 0], 5000).into()).await?;
```

`ThingServer::from_config(&config, &registry)` turns a file into a builder, which can still be changed (to add services, for example).

## The command line

`teta_wot::server::cli::serve_from_cli(&registry)` gives an application's' command line: `-c`/`-j`, `--host`, `--port`, `--fallback`, `--debug`, and exit codes ([Running a server](tutorial/running.md)). `cli::serve_until` takes the arguments, a chance to change the builder, and its own shutdown signal, for tests and [services](windows_service.md).
