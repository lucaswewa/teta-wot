# thing-slots

**Concept:** composing Things. An autofocus uses a stage, a camera and, if there is one, a lamp, without knowing how they are made. The configuration file decides which Things fill its slots, so a simulated camera can be swapped for a real one without changing code.

**Technologies:** `Slot<T>`, `OptSlot<T>`, `SlotMap<T>` and `Slot<dyn Trait>`; `#[wot::interface]` and `#[thing(interfaces(…))]`; `thing_slots` in a configuration; `ThingServer::from_config` with a `ThingRegistry`.

## Run it

```
cargo run -p thing-slots
```

It serves two configurations in turn, focuses with each, then shows a configuration that is refused. It exits with a non-zero status if a result isn't as expected.

| Slot | Type | Filled by |
|---|---|---|
| `stage` | `Slot<Stage>` | the one `Stage` (found by type) |
| `camera` | `Slot<dyn CameraApi>` | the Thing named `camera` (the slot's default), or as configured |
| `lamp` | `OptSlot<Lamp>` | a `Lamp` if there is one, otherwise nothing |
| `cameras` | `SlotMap<dyn CameraApi>` | every Thing providing `CameraApi` |

- **Swapping by configuration.**
  - The first configuration has a `SimCamera` named `camera`.
  - The second has no Thing of that name; its `"thing_slots": {"camera": "real"}` connects the `RealCamera` instead.
  - The same code focuses at 3, then at -2.
- **Interfaces.** `CameraApi` is a trait, declared with `#[wot::interface]`. Each camera lists it in `#[thing(interfaces(CameraApi))]` and implements it for `ThingRef<Self>`, by calling its own action through the generated wrapper. So calls through the interface are validated and locked like HTTP requests.
- **Start order.** The autofocus is listed first, but starts last: Things start after the Things their slots use. With a cycle, configuration order is kept.
