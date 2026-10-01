# device-actor

**Concept:** synchronous hardware drivers behind an asynchronous actor. The driver lives on a dedicated OS thread for its whole life, and every call goes through a bounded mailbox.

**Technologies:** `teta_wot::Device`, `Driver`, `DeviceOptions`, `DeviceState`, and tokio (async callers, `spawn_blocking`-free blocking calls through `call_blocking`).

## Run it

```
cargo run -p device-actor
```

It prints each step and exits with a non-zero status if a check fails. A panic message from the deliberately faulty call is printed too; that is expected.
