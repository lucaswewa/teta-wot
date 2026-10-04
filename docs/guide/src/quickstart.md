# Quickstart

The quickstart counter, written in Rust, and uses it from a browser, and `curl`. It is the [`quickstart-counter`](https://github.com/lucaswewa/teta-wot/tree/main/examples/quickstart-counter) example.

## A Thing

A new binary crate depends on `teta_wot` (see [Installing](tutorial/installing.md)):

```toml
[dependencies]
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
teta-wot = { git = "https://github.com/lucaswewa/teta-wot" }
```

The Thing is a struct and an `impl` block:

```rust,ignore
{{#include ../../../examples/quickstart-counter/src/main.rs:thing}}
```

- **`#[derive(Thing)]`** makes the struct a Thing. Its doc comment becomes the Thing's description.
- **`#[property]`** on a `Prop<T>` field makes a property, which clients read and, unless it's `readonly`, write.
- **`#[thing_impl]`** on the `impl` block makes each `#[action]` method an action, which clients invoke. Doc comments give titles and descriptions.
- **`ActionCtx`** is the invocation's context. `ctx.sleep` waits, but stops early if the invocation is cancelled.

## A server

```rust,ignore
{{#include ../../../examples/quickstart-counter/src/main.rs:server}}
```

and, in an async `main`:

```rust,ignore
{{#include ../../../examples/quickstart-counter/src/main.rs:serve}}
```

`shutdown_signal()` completes on Ctrl-C (and the other ways Windows asks a console program to stop), and the server then stops gracefully.

## Using it

Run it with `cargo run -p quickstart-counter -- --serve`, then:

- **Its Thing Description** is at <http://127.0.0.1:5000/counter/>. It describes the property and the actions, and how to use them.
- **The interactive API documentation** is at <http://127.0.0.1:5000/docs>. "Try it out" invokes `increment_counter`, and reads `counter`.
- **From `curl`:**

  ```text
  curl http://127.0.0.1:5000/counter/counter
  curl -X POST http://127.0.0.1:5000/counter/increment_counter
  ```

  The `POST` answers `201 Created` with the invocation, whose `href` shows its progress.

## Next

- The [tutorial](tutorial/index.md) serves Things from a configuration file, and writes a Thing with more features.
- [Actions](actions.md) and [Properties](properties.md) describe everything these attributes can do.
