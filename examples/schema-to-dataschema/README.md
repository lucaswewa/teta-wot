# schema-to-dataschema

**Concept:** how Rust types become TD `DataSchema`s.

**Technologies:** [schemars](https://docs.rs/schemars) 1.x (`#[derive(JsonSchema)]`, draft 2020-12), and `teta_wot::td::DataSchema::for_type`, which converts schemars' JSON Schema to a TD 1.1 DataSchema. Also `teta_wot::td::Constraints`.

## Run it

```
cargo run -p schema-to-dataschema
```

For each type it prints the JSON Schema that schemars generates, then the DataSchema it converts to. It exits with a non-zero status if a conversion that should work fails, or if the recursive type doesn't fail.
