//! The simulated microscope's server.
//!
//! ```text
//! cargo run -p simulated-microscope                  # a demonstration, in-process, then exit
//! cargo run -p simulated-microscope -- -c examples/simulated-microscope/microscope.json
//! ```
//!
//! With arguments it is a Thing server with TetaThing's command line,
//! serving configuration files that name the microscope's Things; the
//! scripted demo (`demo.py`) drives it with a Python client.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, ensure};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use simulated_microscope::{CONFIG, FOCUS_Z, registry};
use teta_wot::server::{ServerConfig, ThingServer, cli};
use teta_wot::testing::TestClient;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        return cli::serve_from_cli(&registry()).await;
    }
    match demo().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The demonstration: the same steps as `demo.py`, in-process.
async fn demo() -> anyhow::Result<()> {
    let settings = tempfile::tempdir()?;
    let mut config: Value = serde_json::from_str(CONFIG)?;
    config["settings_folder"] = json!(settings.path());
    let config = ServerConfig::from_value(&config)?;
    let server = ThingServer::from_config(&config, &registry())?.build()?;
    println!(
        "Things, in start order: {:?}",
        server.runtime().start_order()
    );
    let client = TestClient::start(server).await?;

    // The stage.
    let moved = invoke(
        &client,
        "/stage/move_to",
        json!({"x": 400, "y": 200, "z": 0}),
    )
    .await?;
    println!("stage.move_to(400, 200, 0) -> {moved}");
    let blurred = client.get("/camera/sharpness").await.json();
    println!(
        "camera.sharpness out of focus: {:.3}",
        blurred.as_f64().unwrap_or_default()
    );

    // A setting, saved to the settings folder.
    client.put_json("/camera/exposure", &json!(30.0)).await;
    let saved =
        std::fs::read_to_string(settings.path().join("camera").join("Settings-Camera.json"))
            .context("the camera's settings file")?;
    println!("camera.exposure = 30, saved: {}", saved.trim());

    // A Blob: a captured image, downloaded.
    let capture = invoke(&client, "/camera/capture", Value::Null).await?;
    let href = capture["href"].as_str().context("a Blob")?;
    let image = client.get(&href.replace("http://testserver", "")).await;
    println!(
        "camera.capture() -> {} ({} bytes of {})",
        capture["media_type"],
        image.body.len(),
        image.header("content-type").unwrap_or("?")
    );
    ensure!(image.body.starts_with(&[0xff, 0xd8]), "a JPEG");

    // Events, over server-sent events.
    let mut focused = client.events("/autofocus/focused").await;

    // The autofocus: two slots, a sweep, an array.
    let focus = invoke(
        &client,
        "/autofocus/run",
        json!({"range": 1000, "steps": 21}),
    )
    .await?;
    let curve = focus["curve"].as_array().context("the curve")?;
    println!(
        "autofocus.run() -> in focus at z = {}, sharpness {:.3}, from {} points",
        focus["z"],
        focus["sharpness"].as_f64().unwrap_or_default(),
        curve.len()
    );
    let z = focus["z"].as_i64().unwrap_or_default();
    ensure!((z - FOCUS_Z).abs() <= 50, "the focus is found");
    println!("  event `focused`: {:?}", focused.next().await);
    let sharp = client.get("/camera/sharpness").await.json();
    println!(
        "camera.sharpness in focus: {:.3}",
        sharp.as_f64().unwrap_or_default()
    );
    ensure!(
        sharp.as_f64() > blurred.as_f64(),
        "focusing sharpens the image"
    );

    // The preview, as MJPEG: two frames.
    let response = client.stream("/camera/preview").await;
    println!(
        "GET /camera/preview: {}",
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
    );
    let mut body = response.into_body();
    let mut received = Vec::new();
    while received.windows(7).filter(|w| w == b"--frame").count() < 3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await?
            .context("the stream ended")??;
        if let Ok(data) = frame.into_data() {
            received.extend_from_slice(&data);
        }
    }
    println!("  two frames received ({} bytes)", received.len());

    client.stop().await;
    Ok(())
}

/// Invokes an action and waits for its output.
async fn invoke(client: &TestClient, path: &str, input: Value) -> anyhow::Result<Value> {
    let body = if input.is_null() { None } else { Some(&input) };
    let invocation = client.post_json(path, body).await.json();
    let status_path = invocation["href"]
        .as_str()
        .context("an invocation")?
        .replace("http://testserver", "");
    loop {
        let status = client.get(&status_path).await.json();
        match status["status"].as_str() {
            Some("completed") => return Ok(status["output"].clone()),
            Some("pending" | "running") => tokio::time::sleep(Duration::from_millis(20)).await,
            _ => anyhow::bail!("{path} failed: {}", status["error"]),
        }
    }
}
