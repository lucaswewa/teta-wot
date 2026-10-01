# cancellable-action

**Concept:** stopping long actions cooperatively. Cancelling an invocation sets a flag; the action notices at its next cancellable sleep or check, and decides what to do.

**Technologies:** `teta_wot::ActionCtx` (`sleep`, `check_cancelled`, `cancelled`, `spawn_child`), `Cancelled`, `InvocationManager::cancel` (what `DELETE /action_invocations/{id}` will call), and `tracing` for the log lines.

## Run it

```
cargo run -p cancellable-action
```

It runs three actions, cancels each after about 50 ms, and prints how each ended. It exits with a non-zero status if an action ends differently from what is expected.
