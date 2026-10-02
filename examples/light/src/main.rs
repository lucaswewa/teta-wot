//! a light with a constrained brightness, a read-only `is_on`, a `toggle`
//! action and a functional `status` property, written with the macros.
//!
//! ```text
//! cargo run -p light              # a short in-process demo
//! cargo run -p light -- --serve   # serve on http://127.0.0.1:5000
//! ```

use std::process::ExitCode;

use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;
use tokio::net::TcpListener;

/// A computer-controlled light, our first example Thing.
#[derive(Thing)]
pub struct Light {
    /// The brightness of the light, in % of maximum.
    #[property(default = 100, ge = 0, le = 100)]
    brightness: Prop<i64>,

    /// Whether the light is currently on.
    #[property(default = false, readonly)]
    is_on: Prop<bool>,
}

#[thing_impl]
impl Light {
    /// Swap the light between on and off.
    #[action]
    async fn toggle(&self) -> Result<(), ActionError> {
        self.is_on.update(|on| *on = !*on)?;
        Ok(())
    }

    /// A human-readable status of the light.
    #[property]
    async fn status(&self) -> String {
        if self.is_on.get() {
            format!("The light is on at {}% brightness.", self.brightness.get())
        } else {
            "The light is off.".to_owned()
        }
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("light", Light::default())
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
    if std::env::args().nth(1).as_deref() == Some("--serve") {
        teta_wot::logging::init(false)?;
        let listener = TcpListener::bind(("127.0.0.1", 5000)).await?;
        println!("listening on http://{}", listener.local_addr()?);
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    let client = TestClient::start(server()?).await?;
    let td = client.get("/light/").await.json();
    let brightness = &td["properties"]["brightness"];
    println!(
        "brightness: {} (minimum {}, maximum {}, default {})",
        brightness["title"], brightness["minimum"], brightness["maximum"], brightness["default"]
    );
    println!(
        "is_on: readOnly {}; status: readOnly {}",
        td["properties"]["is_on"]["readOnly"], td["properties"]["status"]["readOnly"]
    );

    let show = async |label: &str| {
        println!(
            "{label:<34} status → {}",
            client.get("/light/status").await.text()
        );
    };
    show("at first:").await;
    let light = client
        .runtime()
        .thing_ref::<Light>("light")
        .expect("the light");
    light.toggle().await.map_err(ActionError::into_anyhow)?;
    show("after toggle():").await;

    let refused = client.put_json("/light/brightness", &json!(150)).await;
    println!(
        "PUT /light/brightness 150 → {}: {}",
        refused.status,
        refused.json()["detail"][0]["msg"]
    );
    anyhow::ensure!(refused.status == 422);
    client.put_json("/light/brightness", &json!(40)).await;
    show("after PUT /light/brightness 40:").await;
    let read_only = client.put_json("/light/is_on", &json!(false)).await;
    println!("PUT /light/is_on false → {} (read-only)", read_only.status);
    anyhow::ensure!(read_only.status == 405);
    anyhow::ensure!(
        client.get("/light/status").await.json() == "The light is on at 40% brightness."
    );
    client.stop().await;
    Ok(())
}
