# Runtime core

`teta-wot-core` runs Things, with:

- a public builder API (`ThingDefinition`) and a type-erased registry (`ThingHandle`, `PropertyEntry`, `ActionEntry`) that the HTTP binding will call;
- property cells (`Prop<T>`) and functional properties;
- a validator that reproduces pydantic's coerciions and 422 errors, checked against fixtures captured from the reference;
- invocations with a state machine, retention and lazy expirry, and cooperative, consume-on-observe cancellation with child invocations;
- invocation logs captured from `tracing` record.
- the reentrant global lock, the message broker, and device actors running synchronous drivers on their own threads.

| Module | Contents |
|---|---|
| [`thing.rs`] | `Thing` (with `start`/`stop`), `ThingCtx`, `ThingDefinition` |
| [`property.rs`] | `Prop<T>`, `DataProperty`, `FunctionalProperty` (async or blocking getter, setter, resetter, default, constraints), `PropertyEntry`, `PropertyError` |
| [`action.rs`] | `Action`, `ActionError` (anyhow-based), `NoInput` |
| [`invocation.rs`] | `Invocation`, `InvocationStatus`, `InvocationRecord`, `InvocationManager` |
| [`context.rs`] | `InvocationScope`, `ActionCtx` |
| [`cancel.rs`] | `CancelToken`, `Cancelled` |
| [`lock.rs`] | `GlobalLock` |
| [`broker.rs`] | `MessageBroker` |
| [`logs.rs`] | `LogRecord`, `LogBuffer`, `InvocationLogLayer` |
| [`device.rs`] | `Device<D>`, `Driver`, `DeviceOptions`, `DeviceState`, `DeviceError` |
| [`validate.rs`] | `SchemaValidator` |
| [`problem.rs`] | `ProblemDetails` |
| [`runtime.rs`] | `Runtime`/`RuntimeBuilder`, `ThingHandle` |
