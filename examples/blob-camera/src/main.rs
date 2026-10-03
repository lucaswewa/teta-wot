//! Blobs: a camera's captures are returned as JPEG
//! data, from memory and from a file in a temporary directory, downloaded
//! from `/blob/{id}`, and passed back as the input of another action.
//!
//! ```text
//! cargo run -p blob-camera                     # an in-process demo
//! cargo run -p blob-camera -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p blob-camera -- --serve --port 0 # any free port
//! ```
//!

use std::process::ExitCode;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use teta_wot::blob::Jpeg;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;

const WIDTH: u16 = 160;
const HEIGHT: u16 = 120;

/// What `describe` finds in a JPEG.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageInfo {
    /// The size of the file, in bytes.
    bytes: usize,
    /// The width, in pixels.
    width: u16,
    /// The height, in pixels.
    height: u16,
}

/// A simulated camera.
#[derive(Thing)]
pub struct Camera {
    /// How many images have been captured.
    #[property(default = 0, readonly)]
    captures: Prop<i64>,
}

impl Camera {
    /// Takes a picture: a colour gradient with a band that moves each time.
    fn take_picture(&self) -> anyhow::Result<Vec<u8>> {
        let n = self.captures.get();
        self.captures.set(n + 1)?;
        let (w, h) = (usize::from(WIDTH), usize::from(HEIGHT));
        let mut pixels = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let band = (x + n as usize * 16) % w < 20;
                pixels.extend([
                    (x * 255 / w) as u8,
                    (y * 255 / h) as u8,
                    if band { 255 } else { 60 },
                ]);
            }
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 85).encode(
            &pixels,
            WIDTH,
            HEIGHT,
            jpeg_encoder::ColorType::Rgb,
        )?;
        Ok(jpeg)
    }
}

#[thing_impl]
impl Camera {
    /// Capture an image, returned from memory.
    #[action]
    async fn capture(&self) -> Result<Blob<Jpeg>, ActionError> {
        Ok(Blob::from_bytes(self.take_picture()?))
    }

    /// Capture an image, saved to a file in a temporary directory.
    ///
    /// The directory is deleted with the Blob, when the invocation expires.
    #[action]
    async fn capture_to_file(&self) -> Result<Blob<Jpeg>, ActionError> {
        let jpeg = self.take_picture()?;
        let folder = tempfile::tempdir()?;
        std::fs::write(folder.path().join("capture.jpg"), jpeg)?;
        Ok(Blob::from_temporary_directory(folder, "capture.jpg")?)
    }

    /// Describe an image: its size in bytes and in pixels.
    #[action]
    async fn describe(&self, image: Blob<Jpeg>) -> Result<ImageInfo, ActionError> {
        let data = image.bytes().await?;
        let (width, height) =
            jpeg_size(&data).ok_or_else(|| ActionError::handled("not a baseline JPEG"))?;
        Ok(ImageInfo {
            bytes: data.len(),
            width,
            height,
        })
    }
}

/// The width and height in a JPEG's frame header.
fn jpeg_size(data: &[u8]) -> Option<(u16, u16)> {
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xff {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if matches!(data[i + 1], 0xc0..=0xc3) {
            let height = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let width = u16::from_be_bytes([data[i + 7], data[i + 8]]);
            return Some((width, height));
        }
        i += 2 + length;
    }
    None
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("camera", Camera::default())
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

/// Invokes an action and waits for it to finish.
async fn invoke(client: &TestClient, action: &str, input: Option<&Value>) -> anyhow::Result<Value> {
    let created = client.post_json(&format!("/camera/{action}"), input).await;
    anyhow::ensure!(created.status == 201, "{action}: {}", created.text());
    let href = format!(
        "/action_invocations/{}",
        created.json()["id"].as_str().unwrap_or_default()
    );
    loop {
        let invocation = client.get(&href).await.json();
        if !matches!(invocation["status"].as_str(), Some("pending" | "running")) {
            return Ok(invocation);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
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
        println!("listening on http://{}", listener.local_addr()?);
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: what `blob_client.py` does.
    let client = TestClient::start(server()?).await?;
    for action in ["capture", "capture_to_file"] {
        let invocation = invoke(&client, action, None).await?;
        let output = &invocation["output"];
        println!("POST /camera/{action} → output {output}");

        // The link downloads the data; so does the invocation's `/output`.
        let href = output["href"].as_str().unwrap_or_default();
        let path = href.trim_start_matches("http://testserver");
        let image = client.get(path).await;
        println!(
            "GET {path} → {} {}, {} bytes",
            image.status,
            image.header("content-type").unwrap_or("-"),
            image.body.len()
        );
        anyhow::ensure!(image.body.starts_with(&[0xff, 0xd8]));

        // Passing the Blob back: its JSON form (the `href` is what counts).
        let info = invoke(&client, "describe", Some(&json!({ "image": output }))).await?;
        println!(
            "POST /camera/describe {{\"image\": …}} → {}",
            info["output"]
        );
        anyhow::ensure!(info["output"]["width"] == WIDTH && info["output"]["height"] == HEIGHT);
        anyhow::ensure!(info["output"]["bytes"] == image.body.len());
    }
    println!("captures: {}", client.get("/camera/captures").await.text());
    client.stop().await;
    Ok(())
}
