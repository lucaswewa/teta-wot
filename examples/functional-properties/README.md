# functional-properties

**Concept:** properties whose value comes from methods, and what makes them writable, resettable or read-only.

**Technologies:** `#[property]` on getters (with `default`, constraints, `unit`, `readonly` and `blocking`), `#[setter(name)]` and `#[resetter(name)]`.

## Run it

```
cargo run -p functional-properties
```

It prints the TD's view of each property, then the answer to each request, and exits with a non-zero status if one is wrong.
