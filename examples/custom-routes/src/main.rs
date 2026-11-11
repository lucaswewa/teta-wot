//! Router composition: the application's own axum routes and layers, in
//! the same server as its Things, under the same credentials, CORS and
//! graceful shutdown.
//!
//! Beside a lamp Thing, the application serves a health check, and a
//! WebSocket of ticks that ends as soon as the server starts shutting down.
//! A layer stamps every answer, the Things' included. The server needs a
//! bearer token.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use teta_wot::http::ShutdownToken;
use teta_wot::http::axum::Router;
use teta_wot::http::axum::extract::Request;
use teta_wot::http::axum::extract::ws::{Message, WebSocketUpgrade};
use teta_wot::http::axum::middleware::{self, Next};
use teta_wot::http::axum::response::Response;
use teta_wot::http::axum::routing::get;
use teta_wot::prelude::*;
use teta_wot::server::Security;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const TOKEN: &str = "let-me-in";

/// A lamp.
#[derive(Thing)]
pub struct Lamp {
    /// Whether it is on.
    #[property(default = true)]
    on: Prop<bool>,
}

/// Stamps every answer, to show that the layer wraps every route.
async fn stamp(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let stamp = "custom-routes".parse().expect("a valid header value");
    response.headers_mut().insert("x-served-by", stamp);
    response
}

/// A WebSocket that ticks every 100 ms until the server shuts down.
async fn ticks(upgrade: WebSocketUpgrade, shutdown: ShutdownToken) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        for tick in 1.. {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = interval.tick() => {
                    if socket.send(Message::text(format!("tick {tick}"))).await.is_err() {
                        return;
                    }
                }
            }
        }
        let _ = socket.send(Message::Close(None)).await;
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let server = ThingServer::builder()
        .thing("lamp", Lamp::default())
        .security(Security::bearer(TOKEN))
        .merge(
            Router::new()
                .route("/health", get(|| async { "ok" }))
                .route("/ticks", get(ticks)),
        )
        .layer(middleware::from_fn(stamp))
        .build()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?.to_string();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_with(listener, async move {
        let _ = stopped.await;
    }));

    for (path, token) in [
        ("/health", None),
        ("/health", Some(TOKEN)),
        ("/lamp/on", Some(TOKEN)),
    ] {
        let answer = http_get(&address, path, token).await?;
        let status = answer.lines().next().unwrap_or_default();
        let stamped = answer.contains("x-served-by: custom-routes");
        let body = answer.split("\r\n\r\n").nth(1).unwrap_or_default();
        println!(
            "GET {path} (token: {}) → {status}, stamped: {stamped}, {body}",
            token.is_some()
        );
    }

    let mut request = format!("ws://{address}/ticks").into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    for _ in 0..3 {
        if let Some(message) = socket.next().await {
            println!("WebSocket: {}", message?.to_text()?);
        }
    }
    let shutting_down = Instant::now();
    let _ = stop.send(());
    while let Some(Ok(_)) = socket.next().await {}
    println!(
        "WebSocket closed {:?} after the shutdown started",
        shutting_down.elapsed()
    );
    serving.await??;
    println!("server stopped after {:?}", shutting_down.elapsed());
    anyhow::ensure!(
        shutting_down.elapsed() < Duration::from_secs(2),
        "the server waited for the WebSocket"
    );
    Ok(())
}

/// A plain HTTP/1.1 GET, returning the whole answer as text.
async fn http_get(address: &str, path: &str, token: Option<&str>) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect(address).await?;
    let authorization = token.map_or(String::new(), |t| format!("authorization: Bearer {t}\r\n"));
    let request = format!(
        "GET {path} HTTP/1.1\r\nhost: {address}\r\n{authorization}connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await?;
    Ok(answer)
}
