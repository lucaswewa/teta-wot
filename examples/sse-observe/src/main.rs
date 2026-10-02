//! Observing a property in the browser with `EventSource`: a thermometer's
//! reading, pushed as server-sent events. The page finds the stream in the
//! Thing Description (the form with `"subprotocol": "sse"`), as any TD
//! consumer would.
//!
//! ```text
//! cargo run -p sse-observe                     # an in-process demo
//! cargo run -p sse-observe -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p sse-observe -- --serve --port 0 # any free port
//! ```
//!
//! While it serves, open `http://127.0.0.1:5000/thermometer/index.html`,
//! or watch the stream with
//! `curl -N -H "Accept: text/event-stream" http://127.0.0.1:5000/thermometer/temperature`.

use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use teta_wot::http::axum::response::{Html, IntoResponse, Response};
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;
use tokio::task::JoinHandle;

/// How often the simulated sensor is read.
const PERIOD: Duration = Duration::from_millis(250);

/// A thermometer that reads a simulated sensor four times a second.
#[derive(Thing)]
pub struct Thermometer {
    /// The latest reading.
    #[property(default = 21.0, readonly, unit = "degree Celsius")]
    temperature: Prop<f64>,
    /// The reading loop, while the Thing runs.
    reader: Mutex<Option<JoinHandle<()>>>,
}

#[thing_impl]
impl Thermometer {
    /// Starts reading. Every change of the property reaches its observers.
    #[on_start]
    async fn start_reading(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        let this = ctx
            .server()
            .thing_ref::<Thermometer>(ctx.name())
            .ok_or_else(|| anyhow::anyhow!("the thermometer isn't on the server"))?;
        let started = Instant::now();
        let reader = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(PERIOD);
            loop {
                ticks.tick().await;
                let t = started.elapsed().as_secs_f64();
                let reading = 21.0 + 0.8 * (t / 3.0).sin() + 0.05 * (t * 7.0).sin();
                let _ = ThingRef::thing(&this)
                    .temperature
                    .set((reading * 100.0).round() / 100.0);
            }
        });
        *self.reader.lock().unwrap() = Some(reader);
        Ok(())
    }

    /// Stops reading.
    #[on_stop]
    async fn stop_reading(&self) {
        if let Some(reader) = self.reader.lock().unwrap().take() {
            reader.abort();
        }
    }

    /// A page that plots the reading as it arrives.
    #[endpoint(get, "index.html")]
    async fn page(&self) -> Response {
        Html(include_str!("../index.html")).into_response()
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("thermometer", Thermometer::default())
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
        println!("open http://{address}/thermometer/index.html in a browser");
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: what the page does, without a browser.
    let client = TestClient::start(server()?).await?;
    let td: Value = client.get("/thermometer/").await.json();
    let form = td["properties"]["temperature"]["forms"]
        .as_array()
        .and_then(|forms| forms.iter().find(|f| f["subprotocol"] == "sse"))
        .ok_or_else(|| anyhow::anyhow!("no SSE form in the TD"))?;
    println!("The TD's SSE form: {form}");
    let href = form["href"].as_str().unwrap_or_default();

    let mut stream = client.events(href).await;
    println!(
        "GET {href} (Accept: text/event-stream) → {} {}",
        stream.status,
        stream.header("content-type").unwrap_or("-")
    );
    for _ in 0..4 {
        let reading = stream
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("the stream ended"))?;
        println!("data: {reading}");
        anyhow::ensure!(reading.is_f64());
    }

    let page = client.get("/thermometer/index.html").await;
    println!("GET /thermometer/index.html → {}", page.status);
    anyhow::ensure!(page.text().contains("EventSource"));
    client.stop().await;
    Ok(())
}
