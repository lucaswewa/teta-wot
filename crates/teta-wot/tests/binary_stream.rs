//! Binary streams: a `#[binary_stream]` method, its parameters, a slow
//! client, the server's shutdown, and the stream's link in the TD.

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::Deserialize;
use teta_wot::http::Subscriber;
use teta_wot::prelude::*;
use teta_wot::server::ServeError;
use teta_wot::testing::TestClient;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// What a client asks for.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
struct Ask {
    /// The size of each message, in bytes: at least 8.
    #[serde(default)]
    size: usize,
}

/// Counts, a message every millisecond.
#[derive(Thing)]
pub struct Counter {}

#[thing_impl]
impl Counter {
    /// Each message is the count, as 8 bytes, padded to the size asked for.
    #[binary_stream]
    async fn counts(&self, subscriber: Subscriber<Ask>) {
        let mut tick = tokio::time::interval(Duration::from_millis(1));
        for count in 0_u64.. {
            tokio::select! {
                () = subscriber.closed() => return,
                _ = tick.tick() => {
                    // A zeroed allocation: cheap, even in a debug build.
                    let mut message = vec![0_u8; subscriber.params().size.max(8)];
                    message[..8].copy_from_slice(&count.to_le_bytes());
                    subscriber.send(message);
                }
            }
        }
    }
}

/// Serves a counter, until the sender is used or dropped.
async fn serve() -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<Result<(), ServeError>>,
) {
    let server = ThingServer::builder()
        .thing("counter", Counter::default())
        .shutdown_grace(Duration::from_secs(60))
        .build()
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/counter/counts", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_with(listener, async move {
        let _ = stopped.await;
    }));
    (url, stop, serving)
}

/// The next message of the given kind, skipping the others.
async fn next(socket: &mut Socket, binary: bool) -> Message {
    let next = async {
        loop {
            match socket.next().await {
                Some(Ok(message)) if message.is_binary() == binary => return message,
                Some(Ok(_)) => {}
                other => panic!("the stream ended: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), next)
        .await
        .expect("a message within 10 s")
}

fn count(message: &Message) -> u64 {
    u64::from_le_bytes(message.clone().into_data()[..8].try_into().unwrap())
}

#[tokio::test]
async fn parameters_are_validated_then_applied() {
    let (url, _stop, _serving) = serve().await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(next(&mut socket, true).await.len(), 8, "the default size");

    socket
        .send(Message::text(r#"{"size": "big"}"#))
        .await
        .unwrap();
    let reply = next(&mut socket, false).await.into_text().unwrap();
    assert!(reply.contains("invalid parameters"), "{reply}");

    socket
        .send(Message::text(r#"{"size": 1024}"#))
        .await
        .unwrap();
    while next(&mut socket, true).await.len() != 1024 {}
}

#[tokio::test]
async fn a_slow_client_misses_messages_but_the_producer_keeps_going() {
    let (url, _stop, _serving) = serve().await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let size = 1 << 20;
    socket
        .send(Message::text(format!(r#"{{"size": {size}}}"#)))
        .await
        .unwrap();
    while next(&mut socket, true).await.len() != size {}
    // Stop reading for a while. The connection's buffers fill (on Linux they
    // hold several megabytes), then the producer's sink keeps replacing its
    // message: the client will find the messages it missed after the ones
    // the buffers kept.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut counts = vec![count(&next(&mut socket, true).await)];
    let has_gap = |counts: &[u64]| counts.windows(2).any(|w| w[1] > w[0] + 1);
    while counts.len() < 200 && !has_gap(&counts) {
        counts.push(count(&next(&mut socket, true).await));
    }
    assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
    // A producer held up by the client would never skip a count.
    assert!(
        has_gap(&counts),
        "no messages were missed, so the producer waited for the client: {counts:?}"
    );
}

#[tokio::test]
async fn the_stream_closes_when_the_server_shuts_down() {
    let (url, stop, serving) = serve().await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    next(&mut socket, true).await;
    let started = Instant::now();
    stop.send(()).unwrap();
    let within = Duration::from_secs(10);
    tokio::time::timeout(within, async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await
    .expect("the stream closes");
    let served = tokio::time::timeout(within, serving)
        .await
        .expect("the server stops");
    assert!(served.unwrap().is_ok());
    assert!(started.elapsed() < within, "took {:?}", started.elapsed());
}

#[tokio::test]
async fn the_td_links_the_stream_with_its_subprotocol_and_parameters() {
    let server = ThingServer::builder()
        .thing("counter", Counter::default())
        .build()
        .unwrap();
    let client = TestClient::start(server).await.unwrap();
    let td = client.get("/counter/").await.json();
    teta_wot::td::validation::validate_json(&td).expect("a valid TD");
    let links = td["links"].as_array().expect("links");
    let link = links
        .iter()
        .find(|l| {
            l["href"]
                .as_str()
                .is_some_and(|h| h.ends_with("/counter/counts"))
        })
        .expect("a link to the stream");
    assert_eq!(link["rel"], "alternate");
    assert_eq!(link["type"], "application/octet-stream");
    assert_eq!(link["subprotocol"], "websocket");
    assert_eq!(link["parameters"]["properties"]["size"]["type"], "integer");
}
