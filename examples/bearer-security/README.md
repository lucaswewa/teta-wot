# bearer-security

**Concept:** a Thing that requires credentials, here a bearer token, with the requirement described where clients look for it.

**Technologies:**

- `ThingServerBuilder::security(Security::bearer(token))`, or `Security::basic(user, password)`;
- in a configuration file, `"security": {"scheme": "bearer", "token_env": "WOT_TOKEN"}`;
- TD `securityDefinitions`, and OpenAPI `securitySchemes`.

## Run it

```
cargo run -p bearer-security
```

It shows the answers in-process, and exits with a non-zero status if one is wrong. The token is `WOT_TOKEN` from the environment, or made up.

To use it with `curl` or Swagger UI (`http://127.0.0.1:8000/docs`, then "Authorize"):

```
cargo run -p bearer-security -- --serve
```

This prints the `curl` command to use, token included.
