# Concurrency and hardware

`teta-wot` runs Thing code on a tokio runtime: actions and properties are `async`, many run at once, and code that blocks is moved off the runtime's threads.

## What runs at once

- **Requests** are served concurrently: many clients read properties while actions run.
- **Actions** run as background tasks. Several invocations, of the same action or different ones, can run at the same time, unless the global lock stops them.
- **Thing code must therefore be safe to call concurrently.** `Prop<T>`, `Event<T>` and `MjpegStream` are, and other state goes in a `Mutex` or atomics.

## Blocking code

A method that blocks (a vendor SDK call, `std::thread::sleep`, heavy computation) must not run on the async threads.

- **Mark it `blocking`** and write it as `fn`: it then runs on a blocking thread.

  ```rust,ignore
  #[action(blocking)]
  fn measure(&self) -> f64 { self.sdk.read_sensor() }
  ```
- **Inside an async action,** `ctx.blocking(|| …).await` runs a closure on a blocking thread.

## Devices

A driver that must stay on one thread, and whose handle can't be sent between threads (many vendor SDKs; anything holding an `Rc`), goes behind a device actor:

```rust,ignore
#[device(init = SerialInstrument::connect("COM3"))]
port: Device<SerialInstrument>,
```

The driver is created, used and dropped on a thread of its own. Callers queue calls to it:

- `self.port.call(|d| d.query("POS?")).await` from async code;
- `call_blocking` from other threads.

Other features:

- a call can time out;
- a panic in the driver faults the device, and `reset` recovers it;
- `abort` interrupts a long call out of band;
- the device opens before the Thing starts and closes after it stops.

See the [`device-actor`](https://github.com/lucaswewa/teta-wot/tree/main/examples/device-actor) example.

## The global lock

LabThings can serialise hardware access with a global lock (`enable_global_lock`):

- while an action runs, other actions and property writes wait briefly (50 ms) for the lock, then are refused with `GlobalLockBusyError`;
- the lock is reentrant, so an action's in-process calls to other Things hold it too.

`teta-wot` has the same lock, off by default as in LabThings. Actions and properties that don't touch hardware opt out with `global_lock = false`; an action that opted out can still hold it for a critical section with `ctx.hold_global_lock()`. See the [`global-lock`](https://github.com/lucaswewa/teta-wot/tree/main/examples/global-lock) example.

## Cancellation and shutdown

Long actions check for cancellation ([Actions](actions.md#cancellation)), so that a client's `DELETE` and the server's shutdown can stop them. On shutdown:

1. every invocation is cancelled;
2. the server waits up to its grace period for them;
3. it stops the Things in reverse order.

An action that ignores cancellation holds the shutdown for the whole grace period.
