# Settings

A setting is a property that survives restarts: a calibration, an exposure, a stage's speed. Mark a data property `#[setting]` instead of `#[property]`:

```rust,ignore
/// How fast the stage moves, in steps per second.
#[setting(default = 2000.0, gt = 0, le = 100_000)]
speed: Prop<f64>,
```

- **Saving.** After every change, whether a client's write or the Thing's own `set`, the Thing's settings are saved to `{settings_folder}/{thing}/Settings-{Type}.json`. The file is written atomically, so a crash leaves the old one.
- **Loading.** When the server starts, before the Thing's `#[on_start]`, the file is loaded if it exists.
- **Functional properties** can be settings too: `#[setting]` on the getter saves what each write through the property sets.

## The file

 It is indented JSON, one key per setting, with CRLF line endings on Windows:

```json
{
    "speed": 2000.0
}
```

- **An unknown key** is logged, and dropped from the file at the next save.
- **A value that isn't valid** is logged, and its default is used.
- **A file that isn't a JSON object** is logged, and every setting keeps its default. The file is overwritten at the next save.

The server still starts.

## Where

The configuration file's `settings_folder` (default is `./settings`), or `ThingServerBuilder::settings_folder`. A service or anything else started from another folder should use an absolute path.

The [`settings`](https://github.com/lucaswewa/teta-wot/tree/main/examples/settings) example writes, reloads and damages settings files, and shares them with the Python server.
