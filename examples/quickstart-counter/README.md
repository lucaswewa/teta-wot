# quickstart-counter

**Concept:** writing a Thing with the authoring macros: a struct with `#[derive(Thing)]` for the property, and an `impl` block with `#[thing_impl]` for the actions.

**Technologies:** `#[derive(Thing)]`, `#[thing_impl]`, `#[property]`, `#[action]`, `ActionCtx`, the generated `TestThingActions` trait, and `teta_wot::testing::TestClient`.

## Run it

```
cargo run -p quickstart-counter              # a short in-process demo, then exit
cargo run -p quickstart-counter -- --serve   # serve on http://127.0.0.1:5000 until Ctrl-C
```
