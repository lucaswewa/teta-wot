//! The application's own routes and layers in a `ThingServer`: credentials,
//! CORS, layers and the graceful shutdown apply to them as to the Things.

use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::Request;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use futures_util::StreamExt;
use teta_wot_core::{DataProperty, Prop, Thing, ThingDefinition};
use teta_wot_server::testing::TestClient;
use teta_wot_server::{Security, ShutdownToken, ThingServer};
use tokio::net::TcpListener;

struct Lamp {
    on: Prop<bool>,
}

impl Thing for Lamp {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Lamp").property("on", DataProperty::new(|t: &Self| &t.on))
    }
}

fn lamp() -> Lamp {
    Lamp {
        on: Prop::new(true),
    }
}

/// Marks every answer, to show where the layer applies.
async fn mark(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-layer", "yes".parse().unwrap());
    response
}

#[tokio::test]
async fn merged_routes_share_credentials_cors_and_layers_with_the_things() {
    let server = || {
        ThingServer::builder()
            .thing("lamp", lamp())
            .security(Security::bearer("secret"))
            .merge(Router::new().route("/health", get(|| async { "ok" })))
            .layer(middleware::from_fn(mark))
            .build()
            .unwrap()
    };
    let anonymous = TestClient::start(server()).await.unwrap();
    for path in ["/health", "/lamp/on"] {
        assert_eq!(anonymous.get(path).await.status, 401, "{path}");
    }
    let client = TestClient::start(server())
        .await
        .unwrap()
        .with_header("authorization", "Bearer secret")
        .with_header("origin", "http://example.org");
    for path in ["/health", "/lamp/on"] {
        let response = client.get(path).await;
        assert_eq!(response.status, 200, "{path}");
        assert_eq!(response.header("x-layer"), Some("yes"), "{path}");
        assert_eq!(
            response.header("access-control-allow-origin"),
            Some("http://example.org"),
            "{path}"
        );
    }
    assert_eq!(client.get("/health").await.text(), "ok");
}

/// A WebSocket that ticks until the server shuts down.
async fn ticks(upgrade: WebSocketUpgrade, shutdown: ShutdownToken) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        let mut interval = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = interval.tick() => {
                    if socket.send(Message::text("tick")).await.is_err() {
                        return;
                    }
                }
            }
        }
        let _ = socket.send(Message::Close(None)).await;
    })
}

#[tokio::test]
async fn a_merged_websocket_closes_when_the_server_shuts_down() {
    // A grace period far longer than the test allows, so that only the
    // socket closing by itself lets the server stop in time.
    let server = ThingServer::builder()
        .thing("lamp", lamp())
        .shutdown_grace(Duration::from_secs(60))
        .merge(Router::new().route("/ticks", get(ticks)))
        .build()
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_with(listener, async move {
        let _ = stopped.await;
    }));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ticks"))
        .await
        .unwrap();
    assert!(socket.next().await.is_some(), "a first tick");

    let started = Instant::now();
    stop.send(()).unwrap();
    let within = Duration::from_secs(10);
    tokio::time::timeout(within, async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await
    .expect("the socket closes");
    let served = tokio::time::timeout(within, serving)
        .await
        .expect("the server stops");
    assert!(served.unwrap().is_ok());
    assert!(started.elapsed() < within, "took {:?}", started.elapsed());
}
