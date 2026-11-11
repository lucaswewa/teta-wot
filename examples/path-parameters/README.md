# path-parameters

**Concept:** custom endpoints that take values from the path itself. A path segment can be a parameter, `{name}`, and the last segment a wildcard, `{*name}`, which matches the rest of the path, slashes included. When several endpoints match a request, the most specific answers: compared segment by segment, a literal beats a parameter, and a parameter beats a wildcard. Endpoints whose paths differ only in their parameters' names are rejected when the server is built.

**Technologies:** `#[endpoint(get, "tiles/{level}/{tile}")]` and `#[endpoint(get, "files/{*path}")]` in `#[thing_impl]`; the `teta_wot::http::PathParams` extractor (`get`, `parse`), whose values are percent-decoded; the parameters in the OpenAPI document.

## Run it

```
cargo run -p path-parameters
```

It reads two tiles (one at a level the pyramid doesn't have), three files (one through the wildcard with an encoded `/`), and the literal `files/index.txt`. It prints the path parameters that OpenAPI lists, and exits with a non-zero status if an answer is wrong.
