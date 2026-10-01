# invocation-logs

**Concept:** per-invocation logs. Anything an action logs with `tracing`, including from a device closure on another thread, ends up in that invocation's log, which clients see when they poll the invocation.

**Technologies:** `tracing` and `tracing-subscriber`, `teta_wot::logging::init` (console output plus the invocation log layer), `teta_wot::logs::LogRecord`, and `Device` (whose thread re-enters the caller's span).

## Run it

```
cargo run -p invocation-logs
```

It prints two invocations' logs as JSON and the console output of the same events, and exits with a non-zero status if the captured logs aren't as expected.
