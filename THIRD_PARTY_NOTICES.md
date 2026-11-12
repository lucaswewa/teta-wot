# Third-party notices

`teta-wot` is under the MIT licence ([`LICENSE`](LICENSE)). The files below were copied into the repository from other projects, and stay under their own licences.

- **Not in the release binaries.** They contain none of these files. Swagger UI and ReDoc are compiled in only with the `docs-offline` feature, and the TD schema only with `validation`.
- **Crates in the release binaries** are listed with their licence texts in the release package's `THIRD_PARTY_LICENSES.txt` ([`docs/releasing.md`](docs/releasing.md)).

The checksums are SHA-256, of the files as the repository stores them (with LF line endings).

## Swagger UI 5.33.0

- **Where:** `crates/teta-wot-http/assets/swagger-ui-5.33.0/`.
- **From:** the npm package `swagger-ui-dist@5.33.0` ([swagger-api/swagger-ui](https://github.com/swagger-api/swagger-ui)).
- **Licence:** Apache-2.0.
  - `LICENSE` and `NOTICE` (SmartBear Software Inc.) are beside the files.
  - `swagger-ui-bundle.js.LICENSE.txt` holds the notices of the packages bundled into it.
- **Used by:** the docs page at `/docs`, with the `docs-offline` feature.

| File | SHA-256 |
|---|---|
| `swagger-ui-bundle.js` | `62df541529080464a7660adc793eab7128c6193ce3be24ddc1e0e0a4a63edc2f` |
| `swagger-ui-bundle.js.LICENSE.txt` | `63818894e4b04cd0e3180d9cb20761e227a939121e7484f8e1d528227c756f89` |
| `swagger-ui.css` | `1ac324f7dcd27e4b9386b4bd6421271ec147e922a22c05ba24b11515e9aa6321` |
| `oauth2-redirect.js` | `a5c8a34e09e49737382cc5019446eec7c3db3461963fc5bfe448361d3c5549e9` |
| `favicon-32x32.png` | `3ed612f41e050ca5e7000cad6f1cbe7e7da39f65fca99c02e99e6591056e5837` |
| `LICENSE` | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| `NOTICE` | `0d20d1adef18aee3f40dd258172155521ce702ac445cb5f7b7d60ed32dad2fb2` |

## ReDoc 2.5.4

- **Where:** `crates/teta-wot-http/assets/redoc-2.5.4/`.
- **From:** the npm package `redoc@2.5.4`, its `bundles/redoc.standalone.js` ([Redocly/redoc](https://github.com/Redocly/redoc)).
- **Licence:** MIT, copyright Rebilly, Inc.
  - `LICENSE` is beside the file.
  - `redoc.standalone.js.LICENSE.txt` holds the notices of the packages bundled into it.
- **Used by:** the docs page at `/redoc`, with the `docs-offline` feature.

| File | SHA-256 |
|---|---|
| `redoc.standalone.js` | `dcaf76612bc4a3fbcc923a8966dee2f6146a5f32e5ce1b6f02dd60cbbf89500b` |
| `redoc.standalone.js.LICENSE.txt` | `eed602ec32e00b2b9df1984fbcc26a687b1b3193ec01961389c4b8439e6edf73` |
| `LICENSE` | `d3026d549cf68ab7355bcfa85877bf8f845b3334a7efbfdc63936432fb34ff0e` |

## W3C Thing Description 1.1 JSON Schema

- **Where:** `crates/teta-wot-td/schema/td-json-schema-validation.json`.
- **From:** the W3C-hosted copy at <https://www.w3.org/2022/wot/td-schema/v1.1>, version `1.1-12-March-2025`, from the [W3C Web of Things Working Group](https://github.com/w3c/wot-thing-description).
- **Licence:** the [W3C Software and Document License](https://www.w3.org/copyright/software-license-2023/) (2023 version).
- **Used by:** validating Thing Descriptions, with the `validation` feature.
- **SHA-256:** `fd44f319f94c6f16f66978860ef2f9f4e0e9fd7fc5835a2f3840b9cf2002e0f1`.

The licence's short notice, for this file:

> Thing Description JSON Schema: <https://www.w3.org/2022/wot/td-schema/v1.1>
>
> Copyright © 2025 World Wide Web Consortium. All Rights Reserved. This work is distributed under the W3C® Software and Document License [1] in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
>
> [1] <https://www.w3.org/Consortium/Legal/copyright-software>
