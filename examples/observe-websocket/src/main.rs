//! Observing a Thing over its WebSocket, `/{thing}/ws`: a property's changes and an action's
//! status, pushed to the client instead of polled.
//!
//! ```text
//! cargo run -p observe-websocket                     # an in-process demo
//! cargo run -p observe-websocket -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p observe-websocket -- --serve --port 0 # any free port
//! ```
//!
//! While it serves, open `http://127.0.0.1:5000/oven/monitor.html` in a
//! browser, or run `observe.py`.

use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use teta_wot::http::axum::response::{Html, IntoResponse, Response};
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;

/// A small oven, heated in steps.
#[derive(Thing)]
pub struct Oven {
    /// The temperature inside the oven.
    #[property(default = 20.0, readonly, unit = "degree Celsius")]
    temperature: Prop<f64>,
}

#[thing_impl]
impl Oven {
    /// Heat or cool to a temperature, at most 10 degrees per step.
    #[action]
    async fn heat_to(&self, ctx: ActionCtx, target: f64) -> Result<f64, ActionError> {
        loop {
            let now = self.temperature.get();
            if (target - now).abs() < 1e-9 {
                return Ok(now);
            }
            ctx.sleep(Duration::from_millis(200)).await?;
            self.temperature
                .set(now + (target - now).clamp(-10.0, 10.0))?;
        }
    }

    /// Whether the oven is cool enough to open. Computed when read, so it
    /// can't be observed.
    #[property]
    async fn safe_to_open(&self) -> bool {
        self.temperature.get() < 50.0
    }

    /// A page that observes the oven over its WebSocket.
    #[endpoint(get, "monitor.html")]
    async fn monitor(&self) -> Response {
        Html(include_str!("../monitor.html")).into_response()
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("oven", Oven::default())
        .build()?)
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--serve") {
        let port = match args.get(1).map(String::as_str) {
            Some("--port") => args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("--port needs a value"))?
                .parse()?,
            _ => 5000_u16,
        };
        teta_wot::logging::init(false)?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let address = listener.local_addr()?;
        println!("listening on http://{address}");
        println!("open http://{address}/oven/monitor.html in a browser");
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: what the page and `observe.py` do.
    let client = TestClient::start(server()?).await?;
    let mut ws = client.websocket("/oven/ws").await;
    for message in [
        json!({"messageType": "addPropertyObservation", "data": {"temperature": true}}),
        json!({"messageType": "addActionObservation", "data": {"heat_to": true}}),
        // Refused: a functional property isn't observable.
        json!({"messageType": "addPropertyObservation", "data": {"safe_to_open": true}}),
    ] {
        println!("→ {message}");
        ws.send_json(&message).await;
    }
    // The dialect acknowledges nothing, but answers a bad request. As a
    // connection's messages are handled in order, this answer also means
    // the two subscriptions before it are in place.
    let refusal = ws.receive_json().await;
    println!("← {refusal}");
    anyhow::ensure!(refusal["error"]["status"] == "403");

    let invocation = client
        .post_json("/oven/heat_to", Some(&json!({"target": 50})))
        .await;
    println!(
        "POST /oven/heat_to {{\"target\": 50}} → {}",
        invocation.status
    );

    let mut temperatures = Vec::new();
    let mut statuses = Vec::new();
    while statuses.last().map(String::as_str) != Some("completed") {
        let message = ws.receive_json().await;
        println!("← {message}");
        match message["messageType"].as_str() {
            Some("propertyStatus") => temperatures.push(message["data"]["temperature"].clone()),
            Some("actionStatus") => {
                statuses.push(message["data"]["status"].as_str().unwrap_or("").to_owned())
            }
            _ => anyhow::bail!("unexpected message {message}"),
        }
    }
    anyhow::ensure!(temperatures == [json!(30.0), json!(40.0), json!(50.0)]);
    anyhow::ensure!(statuses == ["pending", "running", "completed"]);

    let page = client.get("/oven/monitor.html").await;
    println!(
        "GET /oven/monitor.html → {} ({} bytes of HTML)",
        page.status,
        page.body.len()
    );
    let td: Value = client.get("/oven/").await.json();
    println!(
        "The TD's forms for temperature: {}",
        td["properties"]["temperature"]["forms"]
    );
    ws.close().await;
    client.stop().await;
    Ok(())
}
