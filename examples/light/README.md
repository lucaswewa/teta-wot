# light: with the macros

data properties with constraints and a read-only flag, an action, and a functional property computed by a method.

**Technologies:** `#[derive(Thing)]` with `#[property(default, ge, le, readonly)]`, and `#[thing_impl]` with `#[action]` and a `#[property]` getter.

## Run it

```
cargo run -p light              # a short in-process demo, then exit
cargo run -p light -- --serve   # serve on http://127.0.0.1:5000 until Ctrl-C
```
