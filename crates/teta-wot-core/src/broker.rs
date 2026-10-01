//! Publish/subscribe for property changes, action status and events.
//!
//! Each subscriber has a small buffer (5 messages). The broker never waits:
//! when a subscriber's buffer is full the message is dropped for that subscriber
//! with a warning, and closed subscribers are forgotten on the next publish.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

/// Per-subscriber buffer size.
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
}

/// The subscribers of each (Thing, affordance) pair.
type Subscribers = HashMap<(String, String), Vec<mpsc::Sender<Message>>>;

/// Relays messages to the subscribers of each (Thing, affordance) pair.
#[derive(Debug, Default)]
pub struct MessageBroker {
    subscribers: Mutex<Subscribers>,
}

/// Receives the messages of one subscription. Dropping it unsubscribes.
#[derive(Debug)]
pub struct Subscription {
    receiver: mpsc::Receiver<Message>,
}

impl Subscription {
    /// The next message, or `None` once the broker is gone.
    pub async fn recv(&mut self) -> Option<Message> {
        self.receiver.recv().await
    }

    /// The next message if one is waiting.
    pub fn try_recv(&mut self) -> Option<Message> {
        self.receiver.try_recv().ok()
    }
}

impl MessageBroker {
    /// A broker with no subscribers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribes to the messages of one affordance.
    pub fn subscribe(&self, thing: &str, affordance: &str) -> Subscription {
        let (sender, receiver) = mpsc::channel(SUBSCRIBER_BUFFER);
        self.lock()
            .entry((thing.to_owned(), affordance.to_owned()))
            .or_default()
            .push(sender);
        Subscription { receiver }
    }

    /// Whether anyone is subscribed to the affordance (so that publishers
    /// can skip serialising a payload nobody reads).
    pub fn has_subscribers(&self, thing: &str, affordance: &str) -> bool {
        self.lock()
            .get(&(thing.to_owned(), affordance.to_owned()))
            .is_some_and(|s| s.iter().any(|s| !s.is_closed()))
    }

    /// Sends a message to every subscriber of its affordance, without waiting.
    pub fn publish(&self, message: Message) {
        let mut subscribers = self.lock();
        let key = (message.thing.clone(), message.affordance.clone());
        let Some(senders) = subscribers.get_mut(&key) else {
            return;
        };
        senders.retain(|sender| match sender.try_send(message.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
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
        });
        if senders.is_empty() {
            subscribers.remove(&key);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Subscribers> {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner())
    }
}
