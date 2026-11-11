//! Binary streams: custom endpoints that send binary WebSocket messages,
//! such as camera frames, at high rates, to any number of clients.
//!
//! Each connection gets its own producer, a task running the application's
//! code with a [`Subscriber`]:
//!
//! - **Its parameters,** which the client sets with JSON text messages:
//!   each is deserialised into the parameters' type, and one that doesn't
//!   fit is answered with `{"error": …}` and changes nothing.
//! - **A one-slot sink.** [`send`](Subscriber::send) never waits. A message
//!   that the client hasn't taken yet is replaced by the next one and
//!   counted as dropped, so a slow client gets the newest message and never
//!   holds up the producer or other clients.
//! - **Counters** of messages sent and dropped.
//!
//! The stream ends when the client closes it or the server shuts down. The
//! producer then sees [`closed`](Subscriber::closed) complete, and its task
//! is cancelled if it hasn't returned within a second.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::json;
use teta_wot_td::DataSchema;
use tokio::sync::{Notify, mpsc, watch};

use crate::{Endpoint, ShutdownToken};

/// The media type of a binary stream's messages, in its TD link.
pub const BINARY_STREAM_MEDIA_TYPE: &str = "application/octet-stream";

/// What a connection's producer and its pump share.
#[derive(Default)]
struct Shared {
    /// The message waiting to be sent, if any.
    slot: Mutex<Option<Bytes>>,
    /// Signalled when a message is put in the slot.
    ready: Notify,
    /// True once the connection has ended.
    closed: watch::Sender<bool>,
    sent: AtomicU64,
    dropped: AtomicU64,
}

/// One client's connection to a binary stream, for its producer:
///
/// - **The client's parameters,** a `P`, which it sets with JSON text
///   messages. A message that doesn't deserialise into a `P` is answered
///   with `{"error": …}` and changes nothing.
/// - **A one-slot sink.** [`send`](Self::send) never waits. A message the
///   client hasn't taken yet is replaced by the next one and counted as
///   dropped, so a slow client gets the newest message and never holds up
///   the producer or other clients.
/// - **Counters** of messages sent and dropped.
///
/// The connection ends when the client closes it or the server shuts down.
/// [`closed`](Self::closed) then completes, and [`changed`](Self::changed)
/// returns `false`. The producer's task is cancelled if it hasn't returned
/// within a second.
pub struct Subscriber<P> {
    params: watch::Receiver<P>,
    shared: Arc<Shared>,
}

impl<P: Clone> Subscriber<P> {
    /// The client's current parameters: `P::default()` until it sends some.
    pub fn params(&self) -> P {
        self.params.borrow().clone()
    }

    /// Waits until the client changes its parameters, and returns `false`
    /// if the connection ends instead.
    pub async fn changed(&mut self) -> bool {
        self.params.changed().await.is_ok()
    }

    /// Puts a message in the sink, replacing (and counting as dropped) one
    /// the client hasn't taken yet. Never waits. Returns `false` once the
    /// connection has ended.
    pub fn send(&self, message: impl Into<Bytes>) -> bool {
        if self.is_closed() {
            return false;
        }
        let replaced = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(message.into());
        if replaced.is_some() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.ready.notify_one();
        true
    }

    /// How many messages the client has been sent.
    pub fn sent(&self) -> u64 {
        self.shared.sent.load(Ordering::Relaxed)
    }

    /// How many messages were replaced before the client took them.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Whether the connection has ended.
    pub fn is_closed(&self) -> bool {
        *self.shared.closed.borrow()
    }

    /// Completes when the connection ends: the client closed it, or the
    /// server is shutting down.
    pub async fn closed(&self) {
        let mut closed = self.shared.closed.subscribe();
        let _ = closed.wait_for(|closed| *closed).await;
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> {
    /// A binary stream at `/{thing}/{path}`, a WebSocket of binary messages.
    /// `producer` runs for each connection, with its [`Subscriber`]; its
    /// parameters are a `P`, whose schema the TD link carries, with the
    /// `subprotocol` `websocket` and the media type `application/octet-stream`.
    /// `#[binary_stream]` methods generate these.
    pub fn binary_stream<P, F, Fut>(path: &str, producer: F) -> Self
    where
        P: DeserializeOwned + JsonSchema + Default + Clone + Send + Sync + 'static,
        F: Fn(Arc<T>, Subscriber<P>) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let parameters = DataSchema::for_type::<P>()
            .ok()
            .and_then(|schema| serde_json::to_value(schema).ok());
        let endpoint = Endpoint::new("get", path, move |thing: Arc<T>| {
            move |upgrade: WebSocketUpgrade, shutdown: ShutdownToken| {
                let (thing, producer) = (Arc::clone(&thing), producer.clone());
                async move { upgrade.on_upgrade(move |socket| serve(socket, thing, producer, shutdown)) }
            }
        });
        let endpoint = endpoint
            .link("alternate", Some(BINARY_STREAM_MEDIA_TYPE))
            .link_member("subprotocol", "websocket".into());
        match parameters {
            Some(schema) => endpoint.link_member("parameters", schema),
            None => endpoint,
        }
    }

    fn link_member(mut self, name: &str, value: serde_json::Value) -> Self {
        self.spec = self.spec.link_member(name, value);
        self
    }
}

/// Runs a connection: its producer, and the pump between it and the socket.
async fn serve<T, P, F, Fut>(socket: WebSocket, thing: Arc<T>, producer: F, shutdown: ShutdownToken)
where
    P: DeserializeOwned + Default + Send + Sync + 'static,
    F: Fn(Arc<T>, Subscriber<P>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (params, receiver) = watch::channel(P::default());
    let shared = Arc::new(Shared::default());
    let subscriber = Subscriber {
        params: receiver,
        shared: Arc::clone(&shared),
    };
    let mut task = tokio::spawn(producer(thing, subscriber));
    pump(socket, &shared, &params, &shutdown).await;
    shared.closed.send_replace(true);
    drop(params);
    // The producer sees the end (`closed`, or `changed` returning false), and
    // has a moment to finish, say to log its counters, before it is cancelled.
    if tokio::time::timeout(PRODUCER_GRACE, &mut task)
        .await
        .is_err()
    {
        task.abort();
    }
}

/// How long a producer has to return once its connection has ended.
const PRODUCER_GRACE: Duration = Duration::from_secs(1);

/// How long closing a connection may take: a client that has stopped
/// reading never takes the close frame.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// Sends what the producer puts in the slot, and applies the parameters the
/// client sends, until the client closes or the server shuts down.
///
/// Reading and writing run side by side: a client that stops reading, so
/// that a send waits, is still heard when it closes.
async fn pump<P: DeserializeOwned>(
    socket: WebSocket,
    shared: &Shared,
    params: &watch::Sender<P>,
    shutdown: &ShutdownToken,
) {
    let (mut sink, mut stream) = socket.split();
    // Answers to bad parameters, for the writer to send.
    let (answer, mut answers) = mpsc::channel::<String>(4);
    let read = async {
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Text(text) => match serde_json::from_str::<P>(&text) {
                    Ok(value) => {
                        params.send_replace(value);
                    }
                    Err(error) => {
                        let reply = json!({"error": format!("invalid parameters: {error}")});
                        let _ = answer.try_send(reply.to_string());
                    }
                },
                Message::Close(_) => return,
                _ => {}
            }
        }
    };
    let write = async {
        loop {
            let (message, binary) = tokio::select! {
                () = shared.ready.notified() => {
                    let taken = shared
                        .slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    match taken {
                        Some(message) => (Message::Binary(message), true),
                        None => continue,
                    }
                }
                Some(reply) = answers.recv() => (Message::text(reply), false),
            };
            if sink.send(message).await.is_err() {
                return;
            }
            if binary {
                shared.sent.fetch_add(1, Ordering::Relaxed);
            }
        }
    };
    tokio::select! {
        () = read => {}
        () = write => {}
        () = shutdown.cancelled() => {}
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscriber() -> (Subscriber<u32>, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let (_, params) = watch::channel(0);
        let subscriber = Subscriber {
            params,
            shared: Arc::clone(&shared),
        };
        (subscriber, shared)
    }

    #[test]
    fn a_full_slot_keeps_the_newest_message_and_counts_the_others_dropped() {
        let (subscriber, shared) = subscriber();
        for i in 0..1000_u32 {
            assert!(subscriber.send(Bytes::from(i.to_le_bytes().to_vec())));
        }
        let kept = shared.slot.lock().unwrap().take().unwrap();
        assert_eq!(&kept[..], 999_u32.to_le_bytes());
        assert_eq!((subscriber.sent(), subscriber.dropped()), (0, 999));
    }

    #[test]
    fn nothing_is_sent_after_the_connection_ends() {
        let (subscriber, shared) = subscriber();
        shared.closed.send_replace(true);
        assert!(subscriber.is_closed());
        assert!(!subscriber.send(Bytes::from_static(b"late")));
        assert!(shared.slot.lock().unwrap().is_none());
    }
}
