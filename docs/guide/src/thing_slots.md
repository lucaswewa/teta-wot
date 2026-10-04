# Thing slots

Things often use other Things. An autofocus moves a stage and asks a camera how sharp its image is. `teta-wot` connects them with `#[slot]` fields, filled when the server is built.

```rust,ignore
{{#include ../../../examples/simulated-microscope/src/lib.rs:autofocus}}
```

## Kinds of slot

| Field | Filled with |
|---|---|
| `Slot<T>` | Exactly one Thing |
| `OptSlot<T>` | One Thing, or none |
| `SlotMap<T>` | Every matching Thing, by name |

`T` is a Thing type, or a `dyn` interface.

A slot dereferences to a `ThingRef<T>`, through which the Thing:

- calls the other Thing's actions in-process (`self.stage.move_to(…)`);
- reaches the other Thing itself (`ThingRef::thing(&self.camera).measure()`), for its fields and plain methods.

## How slots are filled

- **By default, by type:** the one Thing of type `T` on the server. If there are none, or several, the server can't be built.
- **`#[slot(default = "camera")]`** names the Thing; `default = ["a", "b"]` names several (for a `SlotMap`); `default = None` leaves an `OptSlot` empty.
- **A configuration file's `thing_slots`** overrides the defaults, per Thing:

  ```json
  {"things": {
      "autofocus": {"class": "microscope.autofocus:Autofocus", "thing_slots": {"camera": "camera2"}},
      "camera": "microscope.camera:Camera",
      "camera2": "microscope.camera:Camera",
      "stage": "microscope.stage:Stage"
  }}
  ```

Slots also set the start order: a Thing starts after the Things in its slots, and stops before them.

## Interfaces

A slot can name what it needs rather than a type, so that a configuration can swap one implementation for another: a simulated camera for a real one. The interface is a trait marked `#[teta_wot::interface]`, and each Thing that provides it lists it in `#[thing(interfaces(…))]` and implements it for `ThingRef<Self>`:

```rust,ignore
/// What an autofocus needs from a camera.
#[teta_wot::interface]
pub trait CameraApi: Send + Sync {
    /// How sharp the image is with the focus at `z`.
    fn sharpness(&self, z: i64) -> BoxFuture<'_, Result<f64, ActionError>>;
}

#[derive(Thing)]
#[thing(interfaces(CameraApi))]
pub struct SimCamera;

impl CameraApi for ThingRef<SimCamera> {
    fn sharpness(&self, z: i64) -> BoxFuture<'_, Result<f64, ActionError>> {
        self.simulate(z)
    }
}

#[derive(Thing)]
pub struct Autofocus {
    #[slot(default = "camera")]
    camera: Slot<dyn CameraApi>,
}
```

The [`thing-slots`](https://github.com/lucaswewa/teta-wot/tree/main/examples/thing-slots) example serves this with two configuration files, one simulated and one "real".

## Services

What isn't a Thing, such as a shared connection, a data store, or a clock that tests can control, can be registered with the server as a service (`ThingServerBuilder::service`), and reached from actions with a `Dep<S>` parameter or from a Thing's context. See the [`dependency-injection`](https://github.com/lucaswewa/teta-wot/tree/main/examples/dependency-injection) example.
