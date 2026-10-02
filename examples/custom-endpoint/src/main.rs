//! Custom endpoints: HTTP routes of a Thing that
//! aren't affordances, written as axum handlers. They don't appear in the
//! Thing Description, but can be the target of links.
//!
//! A data logger serves its readings as a CSV download, a plain-text summary
//! that takes a query parameter, and accepts notes posted as text.

use std::process::ExitCode;
use std::sync::Mutex;

use serde::Deserialize;
use teta_wot::http::axum::extract::Query;
use teta_wot::http::axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use teta_wot::http::axum::http::{HeaderMap, Method, StatusCode};
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// A data logger.
#[derive(Thing)]
pub struct Logger {
    #[thing(init = Mutex::new(vec![(0, 20.5), (60, 21.0), (120, 21.75)]))]
    readings: Mutex<Vec<(u32, f64)>>,
    notes: Mutex<Vec<String>>,
}

/// The query of `summary.txt`.
#[derive(Deserialize)]
struct Unit {
    unit: Option<char>,
}

#[thing_impl]
impl Logger {
    /// How many readings there are.
    #[property]
    async fn count(&self) -> usize {
        self.readings.lock().unwrap().len()
    }

    /// The readings as a CSV file.
    ///
    /// It returns a concrete `Response`: in edition 2024 an `impl
    /// IntoResponse` returned from a method would borrow `&self`.
    #[endpoint(get, "readings.csv")]
    async fn readings_csv(&self) -> Response {
        let mut csv = String::from("seconds,celsius\n");
        for (seconds, celsius) in self.readings.lock().unwrap().iter() {
            csv.push_str(&format!("{seconds},{celsius}\n"));
        }
        (
            [
                (CONTENT_TYPE, "text/csv"),
                (CONTENT_DISPOSITION, "attachment; filename=\"readings.csv\""),
            ],
            csv,
        )
            .into_response()
    }

    /// A one-line summary, in Celsius or (with `?unit=F`) Fahrenheit.
    #[endpoint(get, "summary.txt")]
    async fn summary(&self, Query(query): Query<Unit>) -> String {
        let readings = self.readings.lock().unwrap();
        let mean = readings.iter().map(|(_, c)| c).sum::<f64>() / readings.len() as f64;
        match query.unit {
            Some('F') => format!(
                "{} readings, mean {:.1} °F",
                readings.len(),
                mean * 1.8 + 32.0
            ),
            _ => format!("{} readings, mean {mean:.1} °C", readings.len()),
        }
    }

    /// Adds a note, sent as plain text.
    #[endpoint(post, "notes")]
    async fn add_note(&self, headers: HeaderMap, note: String) -> StatusCode {
        let author = headers
            .get("x-author")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous");
        self.notes.lock().unwrap().push(format!("{author}: {note}"));
        StatusCode::NO_CONTENT
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
    let server = ThingServer::builder()
        .thing("logger", Logger::default())
        .build()?;
    let client = TestClient::start(server).await?;

    let csv = client.get("/logger/readings.csv").await;
    println!("GET /logger/readings.csv → {}", csv.status);
    println!(
        "  content-type: {}",
        csv.header("content-type").unwrap_or("-")
    );
    println!(
        "  content-disposition: {}",
        csv.header("content-disposition").unwrap_or("-")
    );
    print!("{}", csv.text());
    anyhow::ensure!(csv.text().lines().count() == 4);

    for path in ["/logger/summary.txt", "/logger/summary.txt?unit=F"] {
        let summary = client.get(path).await;
        println!("GET {path} → {}: {}", summary.status, summary.text());
    }

    let posted = client
        .request(
            Method::POST,
            "/logger/notes",
            &[("x-author", "Ada")],
            Some(b"Calibrated the probe.".to_vec()),
        )
        .await;
    println!("POST /logger/notes → {}", posted.status);
    anyhow::ensure!(posted.status == StatusCode::NO_CONTENT);
    let logger = client
        .runtime()
        .thing_ref::<Logger>("logger")
        .expect("the logger");
    println!(
        "  notes: {:?}",
        ThingRef::thing(&logger).notes.lock().unwrap()
    );

    let td = client.get("/logger/").await.json();
    println!(
        "The TD lists the property {:?} and no endpoints.",
        td["properties"]
            .as_object()
            .map(|p| p.keys().collect::<Vec<_>>())
    );
    let wrong = client.get("/logger/notes").await;
    println!(
        "GET /logger/notes → {} (allow: {})",
        wrong.status,
        wrong.header("allow").unwrap_or("-")
    );
    anyhow::ensure!(wrong.status == StatusCode::METHOD_NOT_ALLOWED);
    client.stop().await;
    Ok(())
}
