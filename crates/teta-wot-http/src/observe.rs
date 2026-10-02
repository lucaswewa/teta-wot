//! Observation and events: `{prefix}/{thing}/ws`, and server-sent events.
//!
//! The WebSocket:
//!
//! - one broker subscription per connection, which each
//!   `addPropertyObservation`, `addActionObservation` (and, added here,
//!   `addEventSubscription`) message extends;
//! - `propertyStatus`, `actionStatus` (with its `"action name"` key) and
//!   `event` messages;
//! - an error response in the Web Thing Protocol's style for a name that
//!   isn't there (404) or isn't observable (403), after which the rest of
//!   that message's names are skipped;
//! - other message types are ignored.
//!
//! A message that isn't a JSON object with a `messageType`, or whose `data`
//! isn't an object, is logged and ignored.
//!
//! SSE streams carry each value or event's data as JSON in a `data:` line:
//! `GET {prefix}/{thing}/{property}` with `Accept: text/event-stream` for an
//! observable property, and `GET {prefix}/{thing}/{event}` for an event.
//! Streams and WebSockets end when the broker closes at shutdown.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::SecondsFormat;
use http::header::{ACCEPT, UPGRADE};
use http::request::Parts;
use http::{HeaderMap, Method};
use serde_json::{Map, Value, json};
use teta_wot_core::{Message, MessageBroker, MessageKind, Runtime, Subscription, ThingHandle};

/// Where the error `type` URIs of the Web Thing Protocol live.
const WEBTHING_ERROR_URL: &str = "https://w3c.github.io/web-thing-protocol/errors";

/// The WebSocket close code for a server going away (RFC 6455).
const GOING_AWAY: u16 = 1001;

/// How long a server-side close waits for the client's answer.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// The Thing whose WebSocket this request opens, if it is a WebSocket
/// upgrade of `GET {prefix}/{thing}/ws`.
pub(crate) fn websocket_thing(
    runtime: &Runtime,
    prefix: &str,
    parts: &Parts,
) -> Option<Arc<ThingHandle>> {
    let upgrade = parts
        .headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    if parts.method != Method::GET || !upgrade {
        return None;
    }
    let name = parts
        .uri
        .path()
        .strip_prefix(prefix)?
        .strip_prefix('/')?
        .strip_suffix("/ws")?;
    runtime.thing(name).cloned()
}

/// Accepts the WebSocket upgrade and serves the connection.
pub(crate) async fn upgrade(
    broker: Arc<MessageBroker>,
    thing: Arc<ThingHandle>,
    mut parts: Parts,
) -> Response {
    match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(upgrade) => upgrade.on_upgrade(move |socket| serve(socket, broker, thing)),
        Err(rejection) => rejection.into_response(),
    }
}

/// Relays notifications to the socket, and handles what the client sends,
/// until either side closes.
async fn serve(mut socket: WebSocket, broker: Arc<MessageBroker>, thing: Arc<ThingHandle>) {
    let mut subscription = broker.subscription();
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Text(text))) => {
                    if let Some(reply) = handle(&broker, &thing, &subscription, text.as_str())
                        && socket.send(text_message(&reply)).await.is_err()
                    {
                        return;
                    }
                }
                Some(Ok(WsMessage::Binary(_))) => {
                    tracing::error!(
                        thing = %thing.name(),
                        "Got a bad websocket message: a binary message, where JSON text was expected."
                    );
                }
                Some(Ok(WsMessage::Close(_)) | Err(_)) | None => return,
                // Pings are answered by the socket.
                Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {}
            },
            message = subscription.recv() => match message {
                Some(message) => {
                    if socket.send(text_message(&relay(&message))).await.is_err() {
                        return;
                    }
                }
                None => {
                    close(socket).await;
                    return;
                }
            },
        }
    }
}

/// Closes the socket from the server's side, with the close handshake: the
/// client's answer (and anything it sent before) is read before the
/// connection is dropped. A socket dropped with unread data is reset, and
/// the client may lose the close frame (seen on Windows).
async fn close(mut socket: WebSocket) {
    let frame = CloseFrame {
        code: GOING_AWAY,
        reason: "the server is shutting down".into(),
    };
    if socket.send(WsMessage::Close(Some(frame))).await.is_err() {
        return;
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
        while let Some(Ok(message)) = socket.recv().await {
            if matches!(message, WsMessage::Close(_)) {
                break;
            }
        }
    })
    .await;
}

fn text_message(value: &Value) -> WsMessage {
    WsMessage::Text(value.to_string().into())
}

/// What a request can subscribe to.
#[derive(Debug, Clone, Copy)]
enum Target {
    Property,
    Action,
    Event,
}

impl Target {
    fn from_message_type(message_type: &str) -> Option<Self> {
        match message_type {
            "addPropertyObservation" => Some(Self::Property),
            "addActionObservation" => Some(Self::Action),
            "addEventSubscription" => Some(Self::Event),
            _ => None,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Property => "property",
            Self::Action => "action",
            Self::Event => "event",
        }
    }

    /// The `operation` of an error response: `observe{type}`
    /// for properties and actions, the TD operation for events.
    fn operation(self) -> &'static str {
        match self {
            Self::Property => "observeproperty",
            Self::Action => "observeaction",
            Self::Event => "subscribeevent",
        }
    }
}

/// Why a name can't be observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    NotFound,
    NotObservable,
}

/// Handles a client message: subscribes to what it names, and returns the
/// error response to send, if any.
fn handle(
    broker: &MessageBroker,
    thing: &ThingHandle,
    subscription: &Subscription,
    text: &str,
) -> Option<Value> {
    let bad = |why: &str| {
        tracing::error!(thing = %thing.name(), "Got a bad websocket message: {text}, caused {why}.");
    };
    let request: Value = match serde_json::from_str(text) {
        Ok(request) => request,
        Err(error) => {
            bad(&format!("a JSON error: {error}"));
            return None;
        }
    };
    let Some(message_type) = request.get("messageType").and_then(Value::as_str) else {
        bad("a missing messageType");
        return None;
    };
    let target = Target::from_message_type(message_type)?;
    let Some(names) = request.get("data").and_then(Value::as_object) else {
        bad("data that isn't an object");
        return None;
    };
    for name in names.keys() {
        let refusal = match target {
            Target::Property => match thing.property(name) {
                None => Some(Refusal::NotFound),
                Some(property) if !property.is_observable() => Some(Refusal::NotObservable),
                Some(_) => None,
            },
            Target::Action => thing.action(name).is_none().then_some(Refusal::NotFound),
            Target::Event => thing.event(name).is_none().then_some(Refusal::NotFound),
        };
        if let Some(refusal) = refusal {
            bad(&format!("{refusal:?} for '{name}'"));
            return Some(error_response(name, target, refusal));
        }
        broker.add(subscription, thing.name(), name);
    }
    None
}

/// `observation_error_response`.
fn error_response(name: &str, target: Target, refusal: Refusal) -> Value {
    let error = match refusal {
        Refusal::NotFound => json!({
            "status": "404",
            "type": format!("{WEBTHING_ERROR_URL}#not-found"),
            "title": "Not Found",
            "detail": format!("No {} found with the name '{name}'.", target.noun()),
        }),
        Refusal::NotObservable => json!({
            "status": "403",
            "type": format!("{WEBTHING_ERROR_URL}#not-observable"),
            "title": "Not Observable",
            "detail": format!("Property '{name}' is not observable."),
        }),
    };
    json!({
        "messageType": "response",
        "operation": target.operation(),
        "name": name,
        "error": error,
    })
}

/// A notification.
fn relay(message: &Message) -> Value {
    let keyed = |value: Value| {
        let mut data = Map::new();
        data.insert(message.affordance.clone(), value);
        Value::Object(data)
    };
    match message.kind {
        MessageKind::Property => json!({
            "messageType": "propertyStatus",
            "data": keyed(message.payload.clone()),
        }),
        MessageKind::Action => json!({
            "messageType": "actionStatus",
            "data": {"action name": message.affordance, "status": message.payload},
        }),
        // As the Mozilla Web Thing API.
        MessageKind::Event => json!({
            "messageType": "event",
            "data": keyed(json!({
                "data": message.payload,
                "timestamp": message.time.to_rfc3339_opts(SecondsFormat::Micros, true),
            })),
        }),
    }
}

/// Whether the request asks for server-sent events.
pub(crate) fn wants_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get_all(ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|range| {
            range
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("text/event-stream")
        })
}

/// A server-sent events stream of one affordance's messages: each
/// property value or event's data, as JSON.
pub(crate) fn event_stream(broker: &MessageBroker, thing: &str, affordance: &str) -> Response {
    let subscription = broker.subscribe(thing, affordance);
    let events = futures_util::stream::unfold(subscription, |mut subscription| async move {
        let message = subscription.recv().await?;
        let event = SseEvent::default().data(message.payload.to_string());
        Some((Ok::<_, Infallible>(event), subscription))
    });
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_headers() {
        let accept = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(ACCEPT, value.parse().unwrap());
            wants_event_stream(&headers)
        };
        assert!(accept("text/event-stream"));
        assert!(accept("application/json, Text/Event-Stream;q=0.5"));
        assert!(!accept("*/*"));
        assert!(!accept("application/json"));
        assert!(!wants_event_stream(&HeaderMap::new()));
    }

    #[test]
    fn relayed_messages() {
        let message = |kind, payload| Message::new("thing", "name", kind, payload);
        assert_eq!(
            relay(&message(MessageKind::Property, json!(3))),
            json!({"messageType": "propertyStatus", "data": {"name": 3}})
        );
        assert_eq!(
            relay(&message(MessageKind::Action, json!("running"))),
            json!({"messageType": "actionStatus", "data": {"action name": "name", "status": "running"}})
        );
        let event = relay(&message(MessageKind::Event, json!({"a": 1})));
        assert_eq!(event["messageType"], "event");
        assert_eq!(event["data"]["name"]["data"], json!({"a": 1}));
        assert!(
            event["data"]["name"]["timestamp"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
    }

    #[test]
    fn error_responses() {
        assert_eq!(
            error_response("funcprop", Target::Property, Refusal::NotObservable),
            json!({
                "messageType": "response",
                "operation": "observeproperty",
                "name": "funcprop",
                "error": {
                    "status": "403",
                    "type": "https://w3c.github.io/web-thing-protocol/errors#not-observable",
                    "title": "Not Observable",
                    "detail": "Property 'funcprop' is not observable.",
                },
            })
        );
        assert_eq!(
            error_response("missing", Target::Action, Refusal::NotFound)["error"]["detail"],
            "No action found with the name 'missing'."
        );
    }
}
