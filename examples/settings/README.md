# settings

**Concept:** settings, which are properties saved to disk so they survive a restart.

**Technologies:** `#[setting]` on `Prop<T>` fields (and on `#[thing_impl]` getters); `ThingServerBuilder::settings_folder`; the settings file `{folder}/{thing}/Settings-{Class}.json`.

## Run it

```
cargo run -p settings                        # in a temporary folder, deleted afterwards
cargo run -p settings -- --folder settings   # in ./settings, kept
```

It changes settings, restarts the server, and loads a hand-edited file, printing each step. It exits with a non-zero status if a value isn't as expected.
