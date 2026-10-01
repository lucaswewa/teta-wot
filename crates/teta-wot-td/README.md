# TD and schema crate

`teta-wot-td` models W3C TD 1.1 in serde, builds and checks TDs, converts Rust types (through schemars) and any JSON Schema to TD DataSchemas, and validates TDs against a vendored copy of the W3C TD 1.1 JSON Schema.

| Module | Contents |
|---|---|
| [`thing.rs`](src/thing.rs) |  `ThingDescription`, `ThingBuilder`, and `validate()` |
| [`affordance.rs`](src/affordance.rs) | Property, action and event affordances and their builders. |
| [`data_schema.rs`](src/data_schema.rs) | `DataSchema`, `DataType` |
| [`form.rs`](src/form.rs) | Forms and operations |
| [`convert.rs`](src/convert.rs) | JSON Schema -> DataSchema conversion |
| [`constraints.rs`](src/constraints.rs) | `Constraints` |
| [`validation.rs`](src/validation.rs) | W3C JSON Schema validation |
| [`schema/`](schema/td-json-schema-validation.json) | The vendored W3C TD 1.1 JSON Schema |

