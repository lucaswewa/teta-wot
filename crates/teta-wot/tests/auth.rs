//! Pluggable authentication and authorisation: who may do what, the
//! principal reaching every kind of affordance, and WebSockets that
//! authenticate with their first message.

use std::sync::Mutex;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use teta_wot::http::Subscriber;
use teta_wot::http::axum::http::request::Parts;
use teta_wot::http::axum::{Router, routing::get};
use teta_wot::prelude::*;
use teta_wot::server::{Access, Authenticator, Authorizer, Rejection};
use teta_wot::testing::TestClient;
use teta_wot::{BoxFuture, Principal};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// API keys: Alice operates, Bob only looks.
struct Keys;

impl Authenticator for Keys {
    fn authenticate<'a>(
        &'a self,
        request: &'a Parts,
    ) -> BoxFuture<'a, Result<Principal, Rejection>> {
        Box::pin(async move {
            let header = request
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok());
            match header {
                Some("Key alice") => Ok(Principal::new("alice").with_attribute("role", "operator")),
                Some("Key bob") => Ok(Principal::new("bob").with_attribute("role", "viewer")),
                _ => Err(Rejection::unauthorized(Some(
                    "Key realm=\"lab\"".to_owned(),
                ))),
            }
        })
    }
}

/// Viewers may look, but not change anything.
struct ViewersLook;

impl Authorizer for ViewersLook {
    fn authorize(&self, principal: &Principal, access: &Access<'_>) -> bool {
        !access.changes_state || principal.attribute("role") == Some(&json!("operator"))
    }
}

fn name() -> String {
    Principal::current().id().unwrap_or("anonymous").to_owned()
}

/// A notebook that remembers who did what.
#[derive(Thing)]
pub struct Notebook {
    writer: Mutex<String>,
}

#[thing_impl]
impl Notebook {
    /// Who last wrote the note.
    #[property]
    async fn note(&self) -> String {
        self.writer.lock().unwrap().clone()
    }

    #[setter(note)]
    async fn set_note(&self, _value: String) {
        *self.writer.lock().unwrap() = name();
    }

    /// Who invoked it, from the context and from `Principal::current`.
    #[action]
    async fn sign(&self, ctx: ActionCtx) -> String {
        format!("{}/{}", ctx.principal().id().unwrap_or("-"), name())
    }

    /// Who asked.
    #[endpoint(get, "who")]
    async fn who(&self) -> String {
        name()
    }

    /// Sends the client's name, once.
    #[binary_stream]
    async fn names(&self, subscriber: Subscriber<()>) {
        let id = subscriber
            .principal()
            .id()
            .unwrap_or("anonymous")
            .to_owned();
        subscriber.send(id.into_bytes());
        subscriber.closed().await;
    }
}

fn server() -> ThingServer {
    ThingServer::builder()
        .thing("notebook", Notebook::default())
        .authenticator(Keys)
        .authorizer(ViewersLook)
        .merge(Router::new().route("/me", get(|| async { name() })))
        .build()
        .unwrap()
}

async fn as_user(key: &str) -> TestClient {
    let client = TestClient::start(server()).await.unwrap();
    client.with_header("authorization", format!("Key {key}"))
}

#[tokio::test]
async fn unknown_callers_get_401_and_refused_ones_403() {
    let anonymous = TestClient::start(server()).await.unwrap();
    let refused = anonymous.get("/notebook/note").await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.header("www-authenticate"),
        Some("Key realm=\"lab\"")
    );
    assert_eq!(
        anonymous.get("/notebook/").await.status,
        200,
        "TDs stay public"
    );

    let bob = as_user("bob").await;
    assert_eq!(bob.get("/notebook/note").await.status, 200);
    assert_eq!(
        bob.put_json("/notebook/note", &json!("hi")).await.status,
        403
    );
    assert_eq!(bob.post_json("/notebook/sign", None).await.status, 403);
    assert_eq!(
        bob.get("/me").await.status,
        200,
        "a merged route, read only"
    );

    let alice = as_user("alice").await;
    assert!(
        alice
            .put_json("/notebook/note", &json!("hi"))
            .await
            .status
            .is_success()
    );
}

#[tokio::test]
async fn the_principal_reaches_every_kind_of_affordance() {
    let alice = as_user("alice").await;
    alice.put_json("/notebook/note", &json!("hi")).await;
    assert_eq!(
        alice.get("/notebook/note").await.json(),
        json!("alice"),
        "a setter"
    );
    assert_eq!(
        alice.get("/notebook/who").await.text(),
        "alice",
        "an endpoint"
    );
    assert_eq!(alice.get("/me").await.text(), "alice", "a merged route");

    let invocation = alice.post_json("/notebook/sign", None).await.json();
    let href = format!("/action_invocations/{}", invocation["id"].as_str().unwrap());
    let output = loop {
        let record = alice.get(&href).await.json();
        if record["status"] == "completed" {
            break record["output"].clone();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(output, json!("alice/alice"), "an action, and its context");
}

#[tokio::test]
async fn websockets_without_credentials_authenticate_with_their_first_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(server().serve_with(listener, std::future::pending()));
    let connect = |path: &str| {
        let request = format!("ws://{address}{path}")
            .into_client_request()
            .unwrap();
        tokio_tungstenite::connect_async(request)
    };
    let authenticate =
        |key: &str| Message::text(json!({"authorization": format!("Key {key}")}).to_string());

    let (mut stream, _) = connect("/notebook/names").await.unwrap();
    stream.send(authenticate("bob")).await.unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.into_data().as_ref(), b"bob", "the stream runs as bob");

    for path in ["/notebook/names", "/notebook/ws"] {
        let (mut socket, _) = connect(path).await.unwrap();
        socket.send(authenticate("mallory")).await.unwrap();
        match socket.next().await {
            Some(Ok(Message::Close(Some(frame)))) => {
                assert_eq!(u16::from(frame.code), 1008, "{path}")
            }
            other => panic!("{path}: expected a close, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn the_td_describes_the_scheme_of_the_old_security_options() {
    let server = ThingServer::builder()
        .thing("notebook", Notebook::default())
        .security(teta_wot::server::Security::bearer("t"))
        .build()
        .unwrap();
    let td: Value = TestClient::start(server)
        .await
        .unwrap()
        .get("/notebook/")
        .await
        .json();
    assert_eq!(td["security"], json!("bearer_sc"));
}
