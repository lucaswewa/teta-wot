# api-prefix

**Concept:** serving under a path prefix (`api_prefix`), for example behind a reverse proxy that routes `/api/v1` to the server. The example also shows how absolute URLs are built from each request.

**Technologies:** `ThingServerBuilder::api_prefix`, and `teta_wot::testing::TestClient` with a custom `Host`.

## Run it

```
cargo run -p api-prefix
```

It prints what the server writes under the prefix and exits with a non-zero status if a URL is wrong.
