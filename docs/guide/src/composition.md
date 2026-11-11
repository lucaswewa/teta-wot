# Your own routes, layers and TLS

A `ThingServer` can serve more than Things. An application can add its own routes, such as a health check, file downloads or a WebSocket, and its own layers, such as request logging. It can also serve over TLS, with HTTP/2 for clients that negotiate it.

## Merging routes

`ThingServerBuilder::merge` takes an axum `Router`, using the axum that `teta-wot` re-exports, `teta_wot::http::axum`:

```rust,ignore
use teta_wot::http::axum::{Router, routing::get};

let server = ThingServer::builder()
    .thing("lamp", Lamp::default())
    .merge(Router::new().route("/health", get(|| async { "ok" })))
    .build()?;
```

**How requests are routed.** Merged routes are in front of the Things' routes:

- A request whose path a merged route matches goes to it, whatever its method. A method it doesn't answer gets `405`.
- Every other request goes to the Things' routes.
- Don't merge a route at a path that a Thing already uses; it would hide the Thing's route.

**What merged routes share with the Things' routes:**

- **Credentials.** With `security` set, merged routes need the same credentials, and answer `401` without them.
- **CORS headers.**
- **Layers** added with `ThingServerBuilder::layer`.
- **The graceful shutdown.**

**One restriction:** a merged router must not have a fallback.

## Layers

`ThingServerBuilder::layer` adds a tower layer around every route, the Things' and the merged ones, inside CORS. A layer added later wraps the ones added before it:

```rust,ignore
use teta_wot::http::axum::middleware;

let server = ThingServer::builder()
    .thing("lamp", Lamp::default())
    .layer(middleware::from_fn(log_requests))
    .build()?;
```

## Connections that stay open

The graceful shutdown stops accepting connections, then waits up to the grace period for open ones to finish. The Things' own streams end by themselves. A merged handler that holds a connection open, such as a WebSocket, a stream or a long poll, should end it when the shutdown starts.

The `ShutdownToken` extractor (`teta_wot::http::ShutdownToken`) completes when the shutdown starts:

```rust,ignore
async fn ticks(upgrade: WebSocketUpgrade, shutdown: ShutdownToken) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                // … send the next message …
            }
        }
    })
}
```

The token is also available from `ThingServerBuilder::shutdown_token` and `ThingServer::shutdown_token`, and it works in custom endpoints too.

The [`custom-routes`](https://github.com/lucaswewa/teta-wot/tree/main/examples/custom-routes) example has a merged health check, a WebSocket that closes on shutdown, a layer, and credentials.

## TLS and HTTP/2

With the `tls` feature, `teta_wot::server::tls` serves HTTPS. It uses rustls with the `ring` crypto provider.

- **`server_config`** takes the certificate chain and private key, both PEM, and offers HTTP/2, then HTTP/1.1, by ALPN.
- **`TlsListener`** accepts connections and completes each TLS handshake in its own task, so a slow client can't hold up the others.
- **`ThingServer::serve_with_listener`** serves on it, or on any other `axum::serve::Listener`.

```rust,ignore
use teta_wot::server::tls::{TlsListener, server_config};

let config = server_config(&std::fs::read("cert.pem")?, &std::fs::read("key.pem")?)?;
let tcp = tokio::net::TcpListener::bind("0.0.0.0:8443").await?;
server.serve_with_listener(TlsListener::new(tcp, config)?, shutdown_signal()).await?;
```

**Which HTTP version clients get:**

- Clients that negotiate HTTP/2 get it, with many requests sharing one connection. That matters to browsers, which open at most six HTTP/1.1 connections to a server.
- Other clients get HTTP/1.1.
- WebSockets use HTTP/1.1.

**mDNS.** `serve_with_listener` doesn't advertise the server over mDNS; `serve_with` does, on TCP.
