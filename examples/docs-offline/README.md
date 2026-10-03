# docs-offline

**Concept:** the interactive API documentation on a lab network without internet access. With the `docs-offline` feature, Swagger UI (`/docs`) and ReDoc (`/redoc`) are compiled into the server, so their pages load nothing from outside.

**Technologies:** the `docs-offline` feature of `wot`; Swagger UI 5.33.0 and ReDoc 2.5.4, vendored in `crates/wot-http/assets/`; a headless Microsoft Edge driven through the Chrome DevTools Protocol, from Python.

## Run it

```
cargo run -p docs-offline                      # an in-process check, then exit
cargo run -p docs-offline -- --serve           # serve on http://127.0.0.1:5000 until Ctrl-C
```

While it serves, open <http://127.0.0.1:5000/docs> or <http://127.0.0.1:5000/redoc>, even with the network unplugged. Or let a headless Edge use "Try it out", as CI does:
