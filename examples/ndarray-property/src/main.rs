//! `ndarray` arrays on the wire (feature `ndarray`).
//!
//! A camera Thing has a dark frame (a 2-D array property) and an action that
//! takes an image and returns it with the dark frame subtracted, plus one
//! that returns a histogram. On the wire every array is nested lists, one
//! level per dimension. The TD
//! describes a fixed dimension exactly (`NdArray<f64, Ix2>` is an array of
//! arrays of numbers).
//!
//! `cargo run -p ndarray-property` shows it in-process and exits; with
//! `--serve`, it serves on port 5000 (or `--port`) until Ctrl-C, for `curl`.

use std::process::ExitCode;

use anyhow::ensure;
use serde_json::{Value, json};
use teta_wot::ndarray::{Array, Ix1, Ix2, array};
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;

/// A small camera, with arrays for images.
#[derive(Thing)]
pub struct Camera {
    /// The dark frame: what the sensor reads with the shutter closed.
    #[property(default = NdArray(array![[1.0, 2.0, 1.0], [2.0, 3.0, 2.0]]))]
    dark_frame: Prop<NdArray<f64, Ix2>>,
}

#[thing_impl]
impl Camera {
    /// Subtract the dark frame from an image of the same shape.
    #[action]
    async fn correct(&self, image: NdArray<f64, Ix2>) -> Result<NdArray<f64, Ix2>, ActionError> {
        let dark = self.dark_frame.get();
        if image.shape() != dark.shape() {
            return Err(ActionError::handled(format!(
                "the image is {:?}, but the dark frame is {:?}",
                image.shape(),
                dark.shape()
            )));
        }
        Ok(NdArray(image.0 - &dark.0))
    }

    /// Count the pixels of an image in each of `bins` equal bins between
    /// its minimum and maximum. Any number of dimensions.
    #[action]
    async fn histogram(&self, image: NdArray, bins: usize) -> NdArray<u64, Ix1> {
        let (low, high) = image
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &v| {
                (l.min(v), h.max(v))
            });
        let mut counts = Array::zeros(bins.max(1));
        let width = (high - low) / counts.len() as f64;
        for value in image.iter() {
            let bin = if width > 0.0 {
                (((value - low) / width) as usize).min(counts.len() - 1)
            } else {
                0
            };
            counts[bin] += 1;
        }
        NdArray(counts)
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
        .thing("camera", Camera::default())
        .build()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--serve") {
        let port: u16 = match args.iter().position(|a| a == "--port") {
            Some(i) => args.get(i + 1).map_or(Ok(5000), |p| p.parse())?,
            None => 5000,
        };
        println!("serving on http://127.0.0.1:{port}/camera/");
        return Ok(server
            .serve_with(
                tokio::net::TcpListener::bind(("127.0.0.1", port)).await?,
                shutdown_signal(),
            )
            .await?);
    }

    let client = TestClient::start(server).await?;

    let td = client.get("/camera/").await.json();
    println!("The TD describes the dark frame as:");
    let dark = &td["properties"]["dark_frame"];
    println!(
        "  type {}, items {}, default {}",
        dark["type"], dark["items"], dark["default"]
    );
    ensure!(dark["items"]["items"]["type"] == "number");
    println!(
        "and `histogram`'s image, with any number of dimensions: {} branches",
        td["actions"]["histogram"]["input"]["properties"]["image"]["oneOf"]
            .as_array()
            .map_or(0, Vec::len)
    );

    let frame = client.get("/camera/dark_frame").await.json();
    println!("\nGET /camera/dark_frame\n  {frame}");
    ensure!(frame == json!([[1.0, 2.0, 1.0], [2.0, 3.0, 2.0]]));

    let image = json!([[11, 12, 11], [12, 13, 52]]);
    let output = invoke(&client, "correct", json!({ "image": image })).await?;
    println!("POST /camera/correct {{\"image\": {image}}}\n  output {output}");
    ensure!(output == json!([[10.0, 10.0, 10.0], [10.0, 10.0, 50.0]]));

    let output = invoke(
        &client,
        "histogram",
        json!({"image": [[[0, 1], [2, 3]], [[4, 5], [6, 99]]], "bins": 4}),
    )
    .await?;
    println!("POST /camera/histogram, a 2 x 2 x 2 image, 4 bins\n  output {output}");
    ensure!(output == json!([7, 0, 0, 1]));

    // Arrays must be rectangular: numpy's message.
    let ragged = client
        .put_json("/camera/dark_frame", &json!([[1.0, 2.0], [3.0]]))
        .await;
    println!(
        "\nPUT /camera/dark_frame [[1.0, 2.0], [3.0]]\n  {} {}",
        ragged.status.as_u16(),
        ragged.json()["detail"][0]["msg"]
    );
    ensure!(ragged.status == 422);
    Ok(())
}

/// Invokes an action and waits for its output.
async fn invoke(client: &TestClient, action: &str, input: Value) -> anyhow::Result<Value> {
    let invocation = client
        .post_json(&format!("/camera/{action}"), Some(&input))
        .await
        .json();
    let path = invocation["href"]
        .as_str()
        .unwrap_or_default()
        .replace("http://testserver", "");
    loop {
        let status = client.get(&path).await.json();
        match status["status"].as_str() {
            Some("completed") => return Ok(status["output"].clone()),
            Some("pending" | "running") => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            _ => anyhow::bail!("{action} failed: {}", status["error"]),
        }
    }
}
