# custom-endpoint

**Concept:** HTTP routes of a Thing that aren't affordances: a file download, plain text, and a `POST` of raw text. They are served at `/{thing}/{path}` but aren't in the Thing Description.

**Technologies:** `#[endpoint(get, "path")]` in `#[thing_impl]`; axum extractors (`Query`, `HeaderMap`, `String`) and responses (`Response`, `StatusCode`), through `wot::http::axum`.

## Run it

```
cargo run -p custom-endpoint
```

It downloads the CSV, reads the summary in both units, posts a note, and exits with a non-zero status if an answer is wrong.
