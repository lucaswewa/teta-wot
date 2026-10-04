# Documentation

`teta-wot` documents a Thing's API from its code, in two forms: the W3C Thing Description, and an OpenAPI document with interactive pages.

## Doc comments

- **A Thing's doc comment** is its description; its title is the type's name.
- **An affordance's first paragraph** is its title, and the whole comment is its description.

  ```rust,ignore
  /// Find the focus.
  ///
  /// The stage sweeps z over `range` steps around where it is, in `steps`
  /// moves; the camera measures the sharpness at each, and the stage
  /// returns to the sharpest.
  #[action]
  async fn run(&self, …) -> Result<Focus, ActionError> { … }
  ```

  Here the title is "Find the focus.", and the description is both paragraphs.
- **Fields of input and output structs** are described by their own doc comments, which schemars copies into their schemas.
- **`title = "…"` and `description = "…"`** options override the comments.

## The Thing Description

Each Thing's TD is at `/{thing}/`. It is TD 1.1, valid against the W3C schema:

- a stable `id`;
- forms for observation and events;
- top-level forms;
- forms for querying and cancelling invocations;
- links to streams.

Units (`unit = "mm"`), semantic types (`semantic_type = "saref:Temperature"`) and `@context` prefixes (`#[thing(context(saref = "https://saref.etsi.org/core/"))]`) annotate it (the [`units-and-semantics`](https://github.com/lucaswewa/teta-wot/tree/main/examples/units-and-semantics) example).

## OpenAPI and the docs pages

`/openapi.json` is an OpenAPI 3.1 document of every route. It is also served as interactive pages:

- `/docs`, Swagger UI, where "Try it out" sends real requests;
- `/redoc`, ReDoc.

They load pinned versions of their scripts from jsDelivr. The `docs-offline` feature compiles them into the binary for networks without internet access.

`teta_wot::server::openapi` and `thing_description` write both documents without a server, for example to generate a client ([`openapi-export`](https://github.com/lucaswewa/teta-wot/tree/main/examples/openapi-export)).

## Rust documentation

`cargo doc --open -p teta-wot` documents the Rust API; the macros' page lists every attribute and option.
