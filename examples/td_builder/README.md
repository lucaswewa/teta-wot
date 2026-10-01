# td-builder

**Concept:** a W3C Thing Description (TD) built by hand.

**Technologies:** `teta_wot::td` (the `teta-wot-td` crate): `ThingDescription::builder`, the affordance builders, `DataSchema::for_type`, `Constraints`, and validation against the vendored W3C TD 1.1 JSON Schema (`validation` feature). serde_json for output.

## Run it

```
cargo run -p td-builder
```

It prints the TD of an LED illuminator on stdout, then reports on stderr that the TD validates against the W3C schema and that a deliberately broken TD is refused. It exits with a non-zero status if either check fails, which is what the CI smoke test relies on.
