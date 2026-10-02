# units-and-semantics

**Concept:** annotating a Thing Description with units and semantic types.

**Technologies:** `#[thing(semantic_type, context(…))]`, and `unit` and `semantic_type` on `#[property]` and `#[action]`; TD validation against the W3C TD 1.1 JSON Schema (`validation` feature).

## Run it

```
cargo run -p units-and-semantics
```

It prints the TD's `@context`, `@type`s and units, validates the TD, and exits with a non-zero status if the TD is invalid.
