//! A Thing that requires a bearer token.
//!
//! Every interaction needs `Authorization: Bearer <token>`: reading and
//! writing properties, invoking and following actions, observing. The
//! descriptions stay public, so that a client can find out what to send:
//! the TD's `securityDefinitions` and `security` say "bearer token in the
//! `Authorization` header", and so does the OpenAPI document (Swagger UI
//! shows an "Authorize" button).
//!
//! The token comes from the `WOT_TOKEN` environment variable, or is made
//! up and printed. In a configuration file, the same is
//! `"security": {"scheme": "bearer", "token_env": "WOT_TOKEN"}`: secrets are
//! never read from the file.
//!
//! `cargo run -p bearer-security` shows the answers in-process and exits;
//! `--serve` serves on port 8000 (or `--port`) until Ctrl-C, for `curl`.

use std::net::Ipv4Addr;
use std::process::ExitCode;

use anyhow::{Context, ensure};
use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::server::{Security, SecurityConfig, ServerConfig, shutdown_signal};
use teta_wot::testing::TestClient;

/// A safe with a combination.
#[derive(Thing)]
pub struct Safe {
    /// Whether the door is open.
    #[property(default = false)]
    open: Prop<bool>,
}

#[thing_impl]
impl Safe {
    /// Close and lock the door.
    #[action(synchronous)]
    async fn lock(&self) -> Result<(), ActionError> {
        self.open.set(false)?;
        Ok(())
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

fn server(token: &str) -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .security(Security::bearer(token))
        .thing("safe", Safe::default())
        .build()?)
}

async fn run() -> anyhow::Result<()> {
    let token = std::env::var("WOT_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--serve") {
        let port: u16 = match args.iter().position(|a| a == "--port") {
            Some(i) => args.get(i + 1).context("--port needs a number")?.parse()?,
            None => 8000,
        };
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let at = listener.local_addr()?;
        println!("serving on http://{at}/safe/");
        println!("  curl -H \"Authorization: Bearer {token}\" http://{at}/safe/open");
        return Ok(server(&token)?
            .serve_with(listener, shutdown_signal())
            .await?);
    }

    let client = TestClient::start(server(&token)?).await?;

    // Without the token: 401, with a challenge.
    let r = client.get("/safe/open").await;
    println!("GET /safe/open, no token");
    println!(
        "  {} www-authenticate: {}  {}",
        r.status.as_u16(),
        r.header("www-authenticate").unwrap_or("-"),
        r.text()
    );
    ensure!(r.status == 401);
    let r = client.put_json("/safe/open", &json!(true)).await;
    ensure!(r.status == 401, "writes need the token too");

    // The TD is public, and says what to send.
    let td = client.get("/safe/").await.json();
    println!("GET /safe/ (public)");
    println!("  security: {}", td["security"]);
    println!("  securityDefinitions: {}", td["securityDefinitions"]);
    ensure!(td["securityDefinitions"]["bearer_sc"]["scheme"] == "bearer");
    ensure!(td["securityDefinitions"]["bearer_sc"]["in"] == "header");
    let openapi = client.get("/openapi.json").await.json();
    println!(
        "GET /openapi.json (public): securitySchemes {}",
        openapi["components"]["securitySchemes"]
    );

    // A wrong token is refused; the right one works.
    let wrong = client.with_header("authorization", "Bearer not-the-token");
    ensure!(wrong.get("/safe/open").await.status == 401);
    let client = TestClient::start(server(&token)?)
        .await?
        .with_header("authorization", format!("Bearer {token}"));
    let r = client.put_json("/safe/open", &json!(true)).await;
    println!("PUT /safe/open true, with the token: {}", r.status.as_u16());
    ensure!(r.status.is_success());
    let r = client.post_json("/safe/lock", None).await;
    println!("POST /safe/lock, with the token: {}", r.status.as_u16());
    ensure!(r.status.is_success());
    ensure!(client.get("/safe/open").await.json() == false);

    // The same from a configuration file: the token's variable, not the token.
    let config = ServerConfig::from_value(&json!({
        "things": {},
        "security": {"scheme": "bearer", "token_env": "WOT_TOKEN"},
    }))?;
    ensure!(
        config.security
            == Some(SecurityConfig::Bearer {
                token_env: "WOT_TOKEN".into()
            })
    );
    println!(
        "\nA configuration file names the variable: {:?}",
        config.security
    );
    Ok(())
}
