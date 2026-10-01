# counter-server

**Concept:** the first end-to-end milestone.

**Technologies:** `teta_wot::server::ThingServer` (axum underneath), `tets_wot::testing::TestClient` for the in-process demo.

## Run it

```
cargo run -p counter-server                      # a short in-process demo, then exit
cargo run -p counter-server -- --serve           # serve on http://127.0.0.1:5000 until Ctrl-C
```
