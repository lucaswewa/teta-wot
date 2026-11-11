//! Custom authentication and authorisation: the application decides who is
//! calling, and what each caller may do, and its Things know who called.
//!
//! A lab's API keys name its people and their roles. Operators may change
//! things; viewers may only look. A logbook records who wrote each entry.

use std::process::ExitCode;
use std::sync::Mutex;

use serde_json::json;
use teta_wot::http::axum::http::request::Parts;
use teta_wot::prelude::*;
use teta_wot::server::{Access, Authenticator, Authorizer, Rejection};
use teta_wot::testing::TestClient;
use teta_wot::{BoxFuture, Principal};

/// The lab's API keys: `Authorization: Key <key>`.
struct ApiKeys;

impl Authenticator for ApiKeys {
    fn authenticate<'a>(
        &'a self,
        request: &'a Parts,
    ) -> BoxFuture<'a, Result<Principal, Rejection>> {
        Box::pin(async move {
            let key = request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Key "));
            let (name, role) = match key {
                Some("k-ada") => ("ada", "operator"),
                Some("k-ben") => ("ben", "viewer"),
                _ => return Err(Rejection::unauthorized(Some("Key realm=\"lab\"".into()))),
            };
            Ok(Principal::new(name).with_attribute("role", role))
        })
    }
}

/// Operators may do anything; viewers only what changes nothing.
struct Roles;

impl Authorizer for Roles {
    fn authorize(&self, principal: &Principal, access: &Access<'_>) -> bool {
        principal.attribute("role") == Some(&json!("operator")) || !access.changes_state
    }
}

/// A logbook.
#[derive(Thing)]
pub struct Logbook {
    entries: Mutex<Vec<String>>,
}

#[thing_impl]
impl Logbook {
    /// The entries, each signed.
    #[property]
    async fn entries(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }

    /// Adds an entry, signed by whoever asked.
    #[action]
    async fn write(&self, ctx: ActionCtx, text: String) -> usize {
        let author = ctx.principal().id().unwrap_or("anonymous");
        let mut entries = self.entries.lock().unwrap();
        entries.push(format!("{author}: {text}"));
        entries.len()
    }
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
    let server = || {
        ThingServer::builder()
            .thing("logbook", Logbook::default())
            .authenticator(ApiKeys)
            .authorizer(Roles)
            .build()
    };
    for (who, key) in [
        ("nobody", None),
        ("ben", Some("k-ben")),
        ("ada", Some("k-ada")),
    ] {
        let mut client = TestClient::start(server()?).await?;
        if let Some(key) = key {
            client = client.with_header("authorization", format!("Key {key}"));
        }
        let read = client.get("/logbook/entries").await.status;
        let body = json!({"text": format!("{who} was here")});
        let write = client.post_json("/logbook/write", Some(&body)).await.status;
        println!("{who:>6}: reading → {read}, writing → {write}");
        let expected = match key {
            None => (401, 401),
            Some("k-ben") => (200, 403),
            Some(_) => (200, 201),
        };
        anyhow::ensure!(
            (read.as_u16(), write.as_u16()) == expected,
            "{who}: unexpected answers"
        );
        if write.is_success() {
            // The action runs at once; its entry is signed with its caller.
            while client.get("/logbook/entries").await.json() == json!([]) {
                tokio::task::yield_now().await;
            }
            println!(
                "        the logbook: {}",
                client.get("/logbook/entries").await.text()
            );
        }
    }
    Ok(())
}
