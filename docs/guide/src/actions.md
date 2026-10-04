# Actions

An action is something a Thing does when asked: move a stage, capture an image, run an autofocus. Clients invoke it with a `POST`. The server answers at once with an *invocation*, which runs in the background and can be followed, and cancelled.

```rust,ignore
#[thing_impl]
impl Stage {
    /// Move to a position.
    ///
    /// The stage moves in small steps at its speed, and stops where it is if
    /// the move is cancelled.
    #[action(global_lock = false)]
    async fn move_to(&self, x: i64, y: i64, z: i64) -> Result<Position, ActionError> {
        self.travel(Position { x, y, z }).await
    }
}
```

## Inputs and outputs

**Parameters are the input's fields.** `move_to` takes `{"x": 1, "y": 2, "z": 3}`, and its TD describes an object with three required integers. Parameter options:

- `#[param(default = 1000)]` makes a parameter optional, with a default;
- `#[param(default)]` defaults to the type's `Default`;
- `#[param(description = "…")]` describes it.

Some parameters are supplied by the server, not the client:

- `ctx: ActionCtx`, the invocation's context;
- `server: Server`, the server;
- `Dep<S>`, a service registered with the server ([`dependency-injection`](https://github.com/lucaswewa/teta-wot/tree/main/examples/dependency-injection)).

`#[input]` on the only other parameter makes its type the whole input. An action without parameters accepts an empty body, `null` or `{}`.

**The return value is the output.**

- A value, `Result<T, E>` (with `E: Into<ActionError>`), or nothing.
- Any serialisable type with a JSON Schema: numbers, strings, structs (`#[derive(Serialize, JsonSchema)]`), vectors, maps, [Blobs](blobs.md), [arrays](arrays.md).

**Inputs are validated as TetaThing validates them.** pydantic's coercions and error messages are reproduced: `"2"` is accepted for an integer, and an invalid input is a 422 with pydantic's list of errors, before the action runs.

## Invocations

`POST /stage/move_to` answers `201 Created`, with the invocation's URL in `Location` and its JSON in the body. Its `status` goes:

- `pending` → `running` → `completed`, with the `output`;
- or ends `error`, with the problem in `error`;
- or ends `cancelled`.

| Route | What |
|---|---|
| `GET /action_invocations/{id}` | The invocation: status, times, input, output, log, error |
| `DELETE /action_invocations/{id}` | Cancel it |
| `GET /action_invocations/{id}/output` | Its output (a Blob's data, for a Blob) |
| `GET /stage/move_to`, `GET /action_invocations` | The invocations of one action, or of all |

Finished invocations are kept for 5 minutes (`retention = seconds` changes it), then forgotten. In the [`teta-wot` wire profile](wire_profiles.md), invocations are the WoT Profile's `ActionStatus` objects instead.

## Cancellation

Cancellation is cooperative: an action stops where it checks.

- `ctx.sleep(duration)` and `cancellable_sleep(duration)` wait, but return `Err(Cancelled)` as soon as the invocation is cancelled. `?` then ends the action.
- `ctx.check_cancelled()?` checks between steps.
- `ctx.cancelled().await` waits for a cancellation, for use with `tokio::select!`.

A cancelled action ends `cancelled`. Code that must tidy up does it before returning. See the [`cancellable-action`](https://github.com/lucaswewa/teta-wot/tree/main/examples/cancellable-action) example.

## Errors

- **An anticipated failure,** such as "the stage is at its limit", is `Err(ActionError::handled("message"))`: it is logged without a backtrace, and the invocation's `error` has the message.
- **Any other error** converted with `?` (from `anyhow`, `std::io`, …) ends the invocation as `error`, with the error's chain in the log.
- **A panic is caught:** the invocation ends `error`, and the server carries on.

## Logs

`tracing` events emitted while an action runs, on any thread it started, are recorded in its invocation's `log`, which clients read. Call `teta_wot::logging::init` (or `cli::serve_from_cli`, which does) to capture them. See the [`invocation-logs`](https://github.com/lucaswewa/teta-wot/tree/main/examples/invocation-logs) example.

## Options

| Option | Effect |
|---|---|
| `blocking` | The method is synchronous (`fn`, not `async fn`), and runs on a blocking thread |
| `global_lock = false` | Don't hold the server's global lock while running ([Concurrency](concurrency.md)) |
| `retention = 60` | Keep finished invocations 60 seconds |
| `synchronous` | In the `wot` wire profile, the caller waits and gets the output ([Wire profiles](wire_profiles.md)) |
| `title = "…"`, `description = "…"`, `semantic_type = "…"` | Override the doc comment; add an `@type` |

## Calling actions in-process

`#[thing_impl]` generates a trait, `{Thing}Actions`, with one typed method per action, implemented for `ThingRef<{Thing}>`. Other Things call actions through their [slots](thing_slots.md), and tests through the runtime:

```rust,ignore
let stage = runtime.thing_ref::<Stage>("stage").expect("configured");
let position = stage.move_to(0, 0, 250).await?;
```

An in-process call runs as a child of the calling invocation: it shares its cancellation, its log and its hold on the global lock.
