# custom-routes

**Concept:** the application's own routes and layers in the same server as its Things. Merged routes get the server's credentials, CORS, layers and graceful shutdown. A handler that holds a connection open watches the server's shutdown, so a WebSocket closes at once rather than holding up the shutdown for the grace period.

**Technologies:**

- `ThingServerBuilder::merge` with an axum `Router`;
- `ThingServerBuilder::layer` with `axum::middleware::from_fn`;
- the `ShutdownToken` extractor;
- `Security::bearer`;
- `ThingServer::serve_with`;
- a `tokio-tungstenite` WebSocket client.

For TLS and HTTP/2, see the guide's chapter *Your own routes, layers and TLS*, and `crates/teta-wot-server/tests/tls.rs`, which serves with a self-signed certificate.

## Run it

```
cargo run -p custom-routes
```

It shows:

- the merged health check refused without the token, then answering with it;
- a Thing's property answering through the same layer;
- three ticks from the merged WebSocket;
- how quickly the WebSocket and then the server stop once the shutdown starts. It exits with a non-zero status if that takes two seconds or more.
