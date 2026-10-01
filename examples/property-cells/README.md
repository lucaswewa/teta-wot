# property-cells

**Concept:** the typed cells behind data properties, `Prop<T>`: how values are validated, who is notified of changes, and what `read_only` means.

**Technologies:** `teta-wot::Prop` (a `tokio::sync::watch` channel underneath), `Constraints` (`ge`, `le`, `min_length`, `max_length`, `pattern`), the pydantic-compatible validator, and the `MessageBroker`.

## Run it

```
cargo run -p property-cells
```

It prints what each step observes and exits with a non-zero status if any check fails.
