//! Publish/subscribe for property changes, action status and events.
//!
//! A [`Subscription`] is one channel with a small buffer
//! (5 messages), and it can subscribe to several affordances: a WebSocket
//! connection has one channel for everything it observes. The broker never
//! waits: when a subscriber's buffer is full the message is dropped for that
//! subscriber, with a warning, and counted. Closed subscribers are forgotten
//! on the next publish. [`MessageBroker::close`] ends every subscription, so
//! that streams to clients end when the server stops.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

/// per-subscriber buffer size.
pub const SUBSCRIBER_BUFFER: usize = 5;

/// What kind of affordance a message comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    /// A property's new value.
    Property,
    /// An action invocation's new status.
    Action,
    /// An event.
    Event,
}

/// A published message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    /// The Thing's name.
    pub thing: String,
    /// The affordance's name.
    pub affordance: String,
    /// The kind of affordance.
    pub kind: MessageKind,
    /// The property value, the invocation status, or the event data.
    pub payload: Value,
    /// When it was published.
    pub time: DateTime<Utc>,
}

impl Message {
    /// A message published now.
    pub fn new(
        thing: impl Into<String>,
        affordance: impl Into<String>,
        kind: MessageKind,
        payload: Value,
    ) -> Self {
        Self {
            thing: thing.into(),
            affordance: affordance.into(),
            kind,
            payload,
            time: Utc::now(),
        }
    }
}

/// One subscription's channel, as the broker holds it.
#[derive(Debug, Clone)]
struct Subscriber {
    sender: mpsc::Sender<Message>,
    dropped: Arc<AtomicU64>,
}

/// The subscribers of each (Thing, affordance) pair.
type Subscribers = HashMap<(String, String), Vec<Subscriber>>;

/// Relays messages to the subscribers of each (Thing, affordance) pair.
#[derive(Debug)]
pub struct MessageBroker {
    subscribers: Mutex<Subscribers>,
    closed: watch::Sender<bool>,
}

impl Default for MessageBroker {
    fn default() -> Self {
        Self {
            subscribers: Mutex::default(),
            closed: watch::Sender::new(false),
        }
    }
}

/// Receives the messages of the affordances it is subscribed to. Dropping
/// it unsubscribes from all of them.
#[derive(Debug)]
pub struct Subscription {
    subscriber: Subscriber,
    receiver: mpsc::Receiver<Message>,
    closed: watch::Receiver<bool>,
}

impl Subscription {
    /// The next message, or `None` once the broker is closed (or gone) and
    /// the messages already buffered have been received.
    pub async fn recv(&mut self) -> Option<Message> {
        tokio::select! {
            biased;
            message = self.receiver.recv() => message,
            _ = self.closed.wait_for(|closed| *closed) => None,
        }
    }

    /// The next message if one is waiting.
    pub fn try_recv(&mut self) -> Option<Message> {
        self.receiver.try_recv().ok()
    }

    /// How many messages were dropped because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.subscriber.dropped.load(Ordering::Relaxed)
    }
}

impl MessageBroker {
    /// A broker with no subscribers.
    pub fn new() -> Self {
        Self::default()
    }

    /// A subscription to nothing yet: add affordances with
    /// [`add`](Self::add).
    pub fn subscription(&self) -> Subscription {
        let (sender, receiver) = mpsc::channel(SUBSCRIBER_BUFFER);
        Subscription {
            subscriber: Subscriber {
                sender,
                dropped: Arc::default(),
            },
            receiver,
            closed: self.closed.subscribe(),
        }
    }

    /// Subscribes to the messages of one affordance.
    pub fn subscribe(&self, thing: &str, affordance: &str) -> Subscription {
        let subscription = self.subscription();
        self.add(&subscription, thing, affordance);
        subscription
    }

    /// Adds an affordance to a subscription. Returns `false` if it was
    /// already subscribed (it still receives each message once), or if the
    /// broker is closed.
    pub fn add(&self, subscription: &Subscription, thing: &str, affordance: &str) -> bool {
        if self.is_closed() {
            return false;
        }
        let mut subscribers = self.lock();
        let list = subscribers
            .entry((thing.to_owned(), affordance.to_owned()))
            .or_default();
        if list
            .iter()
            .any(|s| s.sender.same_channel(&subscription.subscriber.sender))
        {
            return false;
        }
        list.push(subscription.subscriber.clone());
        true
    }

    /// Whether anyone is subscribed to the affordance (so that publishers
    /// can skip serialising a payload nobody reads).
    pub fn has_subscribers(&self, thing: &str, affordance: &str) -> bool {
        self.lock()
            .get(&(thing.to_owned(), affordance.to_owned()))
            .is_some_and(|s| s.iter().any(|s| !s.sender.is_closed()))
    }

    /// Sends a message to every subscriber of its affordance, without waiting.
    pub fn publish(&self, message: Message) {
        let mut subscribers = self.lock();
        let key = (message.thing.clone(), message.affordance.clone());
        let Some(list) = subscribers.get_mut(&key) else {
            return;
        };
        list.retain(
            |subscriber| match subscriber.sender.try_send(message.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    subscriber.dropped.fetch_add(1, Ordering::Relaxed);
                    // No parent span: this isn't part of the publishing invocation's log.
                    tracing::warn!(
                        parent: None,
                        thing = %message.thing,
                        affordance = %message.affordance,
                        "Could not pass a notification to a subscriber, as its buffer was full.",
                    );
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            },
        );
        if list.is_empty() {
            subscribers.remove(&key);
        }
    }

    /// Ends every subscription (they receive what is already buffered, then
    /// `None`) and refuses new ones. The server closes its broker when it
    /// shuts down, so that WebSocket and SSE streams end.
    pub fn close(&self) {
        self.closed.send_replace(true);
        self.lock().clear();
    }

    /// Whether [`close`](Self::close) was called.
    pub fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// Completes when the broker is closed, which the server does when it
    /// starts shutting down: other long responses (MJPEG streams) end then.
    pub async fn closed(&self) {
        let mut closed = self.closed.subscribe();
        let _ = closed.wait_for(|closed| *closed).await;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Subscribers> {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn message(affordance: &str, payload: Value) -> Message {
        Message::new("t", affordance, MessageKind::Property, payload)
    }

    #[tokio::test]
    async fn one_subscription_receives_several_affordances_once_each() {
        let broker = MessageBroker::new();
        let mut subscription = broker.subscription();
        assert!(broker.add(&subscription, "t", "a"));
        assert!(broker.add(&subscription, "t", "b"));
        assert!(!broker.add(&subscription, "t", "a"), "already subscribed");
        broker.publish(message("a", json!(1)));
        broker.publish(message("b", json!(2)));
        broker.publish(message("c", json!(3)));
        assert_eq!(subscription.recv().await.unwrap().payload, json!(1));
        assert_eq!(subscription.recv().await.unwrap().payload, json!(2));
        assert!(subscription.try_recv().is_none());
    }

    #[tokio::test]
    async fn a_full_buffer_drops_and_counts() {
        let broker = MessageBroker::new();
        let mut subscription = broker.subscribe("t", "a");
        for i in 0..8 {
            broker.publish(message("a", json!(i)));
        }
        assert_eq!(subscription.dropped(), 3);
        for i in 0..5 {
            assert_eq!(subscription.recv().await.unwrap().payload, json!(i));
        }
        assert!(subscription.try_recv().is_none());
    }

    #[tokio::test]
    async fn dropping_a_subscription_unsubscribes() {
        let broker = MessageBroker::new();
        let subscription = broker.subscribe("t", "a");
        assert!(broker.has_subscribers("t", "a"));
        drop(subscription);
        assert!(!broker.has_subscribers("t", "a"));
        broker.publish(message("a", json!(1)));
        assert!(broker.lock().is_empty());
    }

    #[tokio::test]
    async fn closing_ends_subscriptions_after_their_buffer() {
        let broker = MessageBroker::new();
        let mut subscription = broker.subscribe("t", "a");
        broker.publish(message("a", json!(1)));
        broker.close();
        assert_eq!(subscription.recv().await.unwrap().payload, json!(1));
        assert!(subscription.recv().await.is_none());
        let mut late = broker.subscribe("t", "a");
        assert!(!broker.has_subscribers("t", "a"));
        assert!(late.recv().await.is_none());
    }
}
