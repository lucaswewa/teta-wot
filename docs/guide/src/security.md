# Security

By default a server, requires no credentials: anyone who can reach it can use it. A server can require HTTP Basic credentials or a bearer token for every interaction.

```rust,ignore
let server = ThingServer::builder()
    .security(Security::bearer(token))          // or Security::basic("lab", password)
    .thing("stage", Stage::default())
    .build()?;
```

In a configuration file, the secret comes from an environment variable, never from the file itself:

```json
{"security": {"scheme": "bearer", "token_env": "WOT_TOKEN"}}
{"security": {"scheme": "basic", "username": "lab", "password_env": "WOT_PASSWORD"}}
```

## What is protected

- **Protected:** reading and writing properties, invoking and following actions, observing (server-sent events and the WebSocket), streams, Blob downloads, and custom endpoints. Without the credentials, they answer `401` with a `WWW-Authenticate` challenge.
- **Public:** what describes the server, so that clients can find out what to send:
  - Thing Descriptions;
  - `/things/` and `/thing_descriptions/`;
  - discovery;
  - the OpenAPI document and the docs pages.

The TDs say what is required. `securityDefinitions` has `basic_sc` or `bearer_sc`, in the `Authorization` header, and `security` names it. The OpenAPI document has the matching security scheme, so Swagger UI shows an "Authorize" button.

## Limits

- **Credentials travel in clear over HTTP.** On a network you don't trust, put the server behind a proxy that provides TLS.
- **Browsers can't add headers** to `EventSource`, `<img>` (MJPEG) or WebSockets. With Basic, they ask the user and then send the credentials themselves. With a bearer token, WebSockets send it in their first message (see below); `EventSource` and MJPEG need a script or a proxy.

The [`bearer-security`](https://github.com/lucaswewa/teta-wot/tree/main/examples/bearer-security) example shows the answers with and without a token.

## Your own authentication and authorisation

When one password or token isn't enough, such as for several people with different rights, the application decides for itself:

- **An `Authenticator`** says who sent a request: a `Principal`, with an id and attributes such as a role. Or it refuses the request with a `Rejection`: `401`, with a challenge for the `WWW-Authenticate` header.
- **An `Authorizer`** says whether that principal may make the request, from its `Access`. `false` is a `403`. `Access` gives:
  - the Thing;
  - the affordance;
  - the `Operation` (reading a property, invoking an action, observing, a custom endpoint, a merged route, …);
  - whether the request `changes_state`.

```rust,ignore
struct ApiKeys;

impl Authenticator for ApiKeys {
    fn authenticate<'a>(&'a self, request: &'a Parts) -> BoxFuture<'a, Result<Principal, Rejection>> {
        Box::pin(async move {
            match request.headers.get("authorization").and_then(|v| v.to_str().ok()) {
                Some("Key k-ada") => Ok(Principal::new("ada").with_attribute("role", "operator")),
                Some("Key k-ben") => Ok(Principal::new("ben").with_attribute("role", "viewer")),
                _ => Err(Rejection::unauthorized(Some("Key realm=\"lab\"".into()))),
            }
        })
    }
}

struct Roles;

impl Authorizer for Roles {
    fn authorize(&self, principal: &Principal, access: &Access<'_>) -> bool {
        principal.attribute("role") == Some(&json!("operator")) || !access.changes_state
    }
}

let server = ThingServer::builder()
    .authenticator(ApiKeys)
    .authorizer(Roles)
    .thing("logbook", Logbook::default())
    .build()?;
```

- **Either may be left out.** Without an authenticator, `security` authenticates if it is set: the principal is Basic's username, or `bearer`. Otherwise everyone is anonymous, and an authoriser can still refuse them.
- **The same routes stay public.**
- **The TDs** describe the scheme that the authenticator's `security_scheme` gives, if any.

### Who is calling

The code a request runs gets its caller from `Principal::current()`:

- property getters and setters;
- actions, for as long as they run, and `ActionCtx::principal`;
- custom endpoints and merged routes;
- binary-stream producers, and `Subscriber::principal`.

Outside a request, it is anonymous.

### WebSockets

The observation WebSocket and binary streams accept an upgrade request without an `Authorization` header. The socket's first message must then be the credentials:

```json
{"authorization": "Key k-ada"}
```

They are checked as the header would have been. If the check fails, or nothing arrives within 10 seconds, the server closes the socket with code 1008 (a policy violation). A custom WebSocket endpoint can do the same: it marks itself with `EndpointSpec::websocket`, and authenticates the socket with the `teta_wot::http::FirstMessageAuth` it is given.

The [`custom-auth`](https://github.com/lucaswewa/teta-wot/tree/main/examples/custom-auth) example has operators, viewers and a logbook that signs its entries.
