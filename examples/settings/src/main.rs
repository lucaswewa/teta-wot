//! Settings: properties saved to disk.
//!
//! ```text
//! cargo run -p settings                       # a demonstration in a temporary folder
//! cargo run -p settings -- --folder settings  # keep the files in the folder settings
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// How far one stage step moves the image, in micrometres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Calibration {
    x: f64,
    y: f64,
}

/// A microscope camera whose settings survive a restart.
#[derive(Thing)]
pub struct Microscope {
    /// The exposure time, in milliseconds.
    #[setting(default = 10.0, gt = 0, unit = "ms")]
    exposure: Prop<f64>,

    /// The objective in use.
    #[setting(default = "10x")]
    objective: Prop<String>,

    /// The stage calibration.
    #[setting(default = Calibration { x: 1.0, y: 1.0 })]
    calibration: Prop<Calibration>,

    /// Frames captured since the server started: a property, not a setting.
    #[property(default = 0, readonly)]
    frames: Prop<u64>,
}

async fn serve(folder: &Path) -> anyhow::Result<TestClient> {
    let server = ThingServer::builder()
        .thing("microscope", Microscope::default())
        .settings_folder(folder)
        .build()?;
    Ok(TestClient::start(server).await?)
}

async fn show(client: &TestClient) {
    for name in ["exposure", "objective", "calibration"] {
        println!(
            "  {name}: {}",
            client.get(&format!("/microscope/{name}")).await.text()
        );
    }
}

fn settings_file(folder: &Path) -> PathBuf {
    folder.join("microscope").join("Settings-Microscope.json")
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
    let _ = teta_wot::logging::init(false);
    let args: Vec<String> = std::env::args().collect();
    let temporary = tempfile::tempdir()?;
    let folder = match args.iter().position(|a| a == "--folder") {
        Some(i) => PathBuf::from(
            args.get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("--folder needs a path"))?,
        ),
        None => temporary.path().to_owned(),
    };

    println!("1. A new server has its defaults, and no settings file yet:");
    let client = serve(&folder).await?;
    show(&client).await;
    println!("  file exists: {}", settings_file(&folder).exists());

    println!("\n2. Every change is saved:");
    client.put_json("/microscope/exposure", &json!(25)).await;
    client
        .put_json("/microscope/objective", &json!("40x"))
        .await;
    println!("{}", std::fs::read_to_string(settings_file(&folder))?);
    client.stop().await;

    println!("\n3. A restarted server loads them before starting:");
    let client = serve(&folder).await?;
    show(&client).await;
    anyhow::ensure!(client.get("/microscope/exposure").await.json() == 25.0);
    client.stop().await;

    println!("\n4. A hand-edited file: unknown keys and invalid values only warn.");
    std::fs::write(
        settings_file(&folder),
        r#"{"exposure": -5, "objective": "100x", "lamp": "on"}"#,
    )?;
    let client = serve(&folder).await?;
    show(&client).await;
    anyhow::ensure!(client.get("/microscope/objective").await.json() == "100x");
    anyhow::ensure!(client.get("/microscope/exposure").await.json() == 10.0);
    client.stop().await;

    println!("\nThe settings are in {}", folder.display());
    Ok(())
}
