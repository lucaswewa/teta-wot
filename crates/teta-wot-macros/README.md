# Authoring macros

Things are now written with two macros:

- `#[derive(Thing)]` on the struct, for data properties, devices and construction from a typed configuration;
- `#[thing_impl]` on its `impl` block, for actions, functional properties with setters and resetters, custom endpoints and lifecycle hooks.
