//! Functional properties: properties whose value a method computes
//!
//! A temperature controller shows each kind:
//!
//! - `setpoint`: a getter, a setter and a resetter, a default and
//!   constraints, which are checked before the setter runs;
//! - `temperature`: a `blocking` getter, for a slow synchronous sensor read;
//! - `heating`: a getter only, so read-only;
//! - `power_limit`: `readonly` with a setter, which Rust code may call but
//!   clients may not.

use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// A temperature controller with a simulated heater.
#[derive(Thing)]
pub struct Controller {
    #[thing(init = Mutex::new(20.0))]
    setpoint: Mutex<f64>,
    #[thing(init = Mutex::new(100))]
    power_limit: Mutex<u8>,
}

#[thing_impl]
impl Controller {
    /// The target temperature.
    #[property(default = 20.0, ge = 5, le = 30, unit = "degree Celsius")]
    async fn setpoint(&self) -> f64 {
        *self.setpoint.lock().unwrap()
    }

    #[setter(setpoint)]
    async fn set_setpoint(&self, value: f64) {
        *self.setpoint.lock().unwrap() = value;
    }

    #[resetter(setpoint)]
    async fn reset_setpoint(&self) {
        *self.setpoint.lock().unwrap() = 20.0;
    }

    /// The measured temperature.
    ///
    /// The sensor is read synchronously and slowly, so the getter is marked
    /// `blocking` and runs on a blocking thread.
    #[property(blocking, unit = "degree Celsius")]
    fn temperature(&self) -> Result<f64, PropertyError> {
        std::thread::sleep(Duration::from_millis(20));
        Ok(18.5)
    }

    /// Whether the heater is on.
    #[property]
    async fn heating(&self) -> bool {
        18.5 < *self.setpoint.lock().unwrap()
    }

    /// The heater's power limit, in percent. Only the Thing's code sets it.
    #[property(readonly, default = 100)]
    async fn power_limit(&self) -> u8 {
        *self.power_limit.lock().unwrap()
    }

    #[setter(power_limit)]
    async fn set_power_limit(&self, value: u8) {
        *self.power_limit.lock().unwrap() = value;
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
        .thing("controller", Controller::default())
        .build()?;
    let client = TestClient::start(server).await?;

    let td = client.get("/controller/").await.json();
    for name in ["setpoint", "temperature", "heating", "power_limit"] {
        let p = &td["properties"][name];
        println!(
            "{name:<12} readOnly {:<5} default {:<5} unit {}",
            p["readOnly"].to_string(),
            p["default"].to_string(),
            p["unit"]
        );
    }

    let step = async |label: String, status: u16, expected: u16| -> anyhow::Result<()> {
        println!("{label:<38} → {status}");
        anyhow::ensure!(status == expected, "expected {expected}");
        Ok(())
    };
    let put = client
        .put_json("/controller/setpoint", &json!(25))
        .await
        .status
        .as_u16();
    step("PUT setpoint 25 (setter runs)".into(), put, 201).await?;
    let read = client.get("/controller/setpoint").await.json();
    println!("GET setpoint                           → {read}");
    anyhow::ensure!(read == 25.0);
    let put = client
        .put_json("/controller/setpoint", &json!(40))
        .await
        .status
        .as_u16();
    step("PUT setpoint 40 (above le = 30)".into(), put, 422).await?;
    let reset = client
        .post_json("/controller/setpoint/reset", None)
        .await
        .status
        .as_u16();
    step("POST setpoint/reset (resetter runs)".into(), reset, 200).await?;
    anyhow::ensure!(client.get("/controller/setpoint").await.json() == 20.0);

    println!(
        "GET temperature (blocking getter)      → {}",
        client.get("/controller/temperature").await.text()
    );
    let put = client
        .put_json("/controller/heating", &json!(true))
        .await
        .status
        .as_u16();
    step("PUT heating (no setter)".into(), put, 405).await?;
    let put = client
        .put_json("/controller/power_limit", &json!(50))
        .await
        .status
        .as_u16();
    step("PUT power_limit (readonly)".into(), put, 405).await?;

    // The Thing's own code may still use the setter.
    let controller = client
        .runtime()
        .thing_ref::<Controller>("controller")
        .expect("the controller");
    ThingRef::thing(&controller).set_power_limit(50).await;
    let limit = client.get("/controller/power_limit").await.json();
    println!("set_power_limit(50) from Rust, then GET power_limit → {limit}");
    anyhow::ensure!(limit == 50);
    client.stop().await;
    Ok(())
}
