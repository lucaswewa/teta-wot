# global-lock

**Concept:** Optional global lock, which makes actions and property writes happen one at a time.

**Technologies:** `RuntimeBuilder::global_lock`, `Action::global_lock(false)`, `ActionCtx::hold_global_lock`, and the problem details that busy requests get.

## Run it

```
cargo run -p global-lock
```

It prints what each request gets while one action holds the lock, and exits with a non-zero status if anything differs from what is expected.
