//! Dependency injection: a shared service, registered once with
//! the server, is injected into actions as a `Dep<T>` parameter. A `Server`
//! parameter gives the action the server: here, to embed every Thing's
//! state (`thing_states()`) and the application configuration in each
//! record.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// A record store shared by every Thing.
#[derive(Default)]
pub struct DataStore {
    records: Mutex<Vec<Value>>,
}

impl DataStore {
    fn add(&self, record: Value) -> usize {
        let mut records = self.records.lock().unwrap();
        records.push(record);
        records.len()
    }
}

/// A thermometer.
#[derive(Thing)]
pub struct Thermometer {
    /// The temperature, in degrees Celsius.
    #[property(default = 21.5, readonly)]
    temperature: Prop<f64>,
}

#[thing_impl]
impl Thermometer {
    /// Record the temperature.
    #[action]
    async fn log(&self, store: Dep<DataStore>) -> usize {
        store.add(json!({"temperature": self.temperature.get()}))
    }

    /// What other Things' records should say about this one.
    #[thing_state]
    fn state(&self) -> Value {
        json!({"temperature": self.temperature.get()})
    }
}

/// A camera.
#[derive(Thing)]
pub struct Camera {
    /// The exposure time, in milliseconds.
    #[property(default = 10.0)]
    exposure: Prop<f64>,
}

#[thing_impl]
impl Camera {
    /// Capture an image, and record it with the state of every Thing.
    #[action]
    async fn capture(&self, store: Dep<DataStore>, server: Server) -> usize {
        store.add(json!({
            "image": format!("frame-{}.jpg", store.records.lock().unwrap().len()),
            "metadata": server.thing_states(),
            "lab": server.application_config(),
        }))
    }

    #[thing_state]
    fn state(&self) -> Value {
        json!({"exposure": self.exposure.get()})
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
    let store = Arc::new(DataStore::default());
    let server = ThingServer::builder()
        .thing("thermometer", Thermometer::default())
        .thing("camera", Camera::default())
        .service(Arc::clone(&store))
        .application_config(json!({"lab": "B12"}))
        .build()?;
    let client = TestClient::start(server).await?;

    // Over HTTP, `Dep<DataStore>` and `Server` aren't part of the input.
    let td = client.get("/camera/").await.json();
    println!(
        "capture's input schema: {}",
        td["actions"]["capture"]["input"]["oneOf"][0]
    );

    for path in ["/thermometer/log", "/camera/capture"] {
        let href = client.post_json(path, None).await.json()["href"]
            .as_str()
            .unwrap_or_default()
            .replace("http://testserver", "");
        while matches!(
            client.get(&href).await.json()["status"].as_str(),
            Some("pending" | "running")
        ) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
    // In-process calls get the same injection.
    let camera = client
        .runtime()
        .thing_ref::<Camera>("camera")
        .expect("added");
    camera.capture().await.map_err(ActionError::into_anyhow)?;

    let records = store.records.lock().unwrap().clone();
    println!("\nThe store holds {} records:", records.len());
    for record in &records {
        println!("  {record}");
    }
    anyhow::ensure!(records.len() == 3);
    anyhow::ensure!(records[1]["metadata"]["thermometer"]["temperature"] == 21.5);
    client.stop().await;

    // A Thing that needs a service can't be built without it.
    let error = ThingServer::builder()
        .thing("camera", Camera::default())
        .build()
        .unwrap_err();
    println!("\nWithout the service: {error}");
    Ok(())
}
