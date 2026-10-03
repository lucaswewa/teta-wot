# openapi-export

**Concept:** describing a server's API without running it. The example builds a runtime (it doesn't start it) and writes the OpenAPI 3.1 document and every Thing Description to files. A standard code generator then makes a TypeScript client from the document.

**Technologies:** `teta_wot::server::openapi` and `teta_wot::server::thing_description`; `HttpOptions` (the API's title and version); `openapi-typescript`, run with `npx`.

## Run it

```
cargo run -p openapi-export                                     # writes to target/openapi-export
cargo run -p openapi-export -- out --base http://lab-pc:5000/   # another folder, TDs for another host
```

It writes `openapi.json`, `stage.td.json` and `camera.td.json`. Then generate the client's types (Node.js needed):

```
npx --yes openapi-typescript@7 target/openapi-export/openapi.json -o target/openapi-export/api.d.ts
```

With [`openapi-fetch`](https://openapi-ts.dev/openapi-fetch/), the types make a typed client:

```ts
import createClient from "openapi-fetch";
import type { paths } from "./api";

const client = createClient<paths>({ baseUrl: "http://localhost:5000" });
const { data } = await client.POST("/stage/move_to", { body: { x: 1, y: 2 } });
```
