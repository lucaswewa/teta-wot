# builder-thing

**Concept:** a Thing defined with the builder API.

**Technologies:** `teta-wot::ThingDefinition`, `DataProperty` and `FunctionalProperty`, `Action` with typed input and output, `NoInput`, `Thing::start`/`stop`, `Runtime` and its registry (`ThingHandle`, `PropertyEntry`, `ActionEntry`), and the TD generated from the registry. serde and schemars describe the data.

## Run it

```
cargo run -p builder-thing
```

It prints an invocation record and the Thing Description on stdout, and a few notes on stderr. It exits with a non-zero status if anything behaves differently from what the code asserts.
