//! MJPEG streams: a simulated camera
//! thread feeds JPEG frames into a stream, which a browser plays at
//! `/camera/preview/viewer`. Actions wait for frames: one measures the
//! next frame's size (an autofocus sharpness measure), another saves one.
//!
//! ```text
//! cargo run -p mjpeg-stream                     # an in-process demo
//! cargo run -p mjpeg-stream -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p mjpeg-stream -- --serve --port 0 # any free port
//! ```
//!
//! While it serves, open `http://127.0.0.1:5000/camera/preview/viewer`.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use http_body_util::BodyExt;
use serde_json::Value;
use teta_wot::blob::Jpeg;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;

/// A simulated camera with a live preview.
#[derive(Thing)]
pub struct Camera {
    /// The live preview, at `/camera/preview`.
    #[stream]
    preview: MjpegStream,

    /// Frames per second of the preview.
    #[property(default = 10.0, gt = 0, le = 30)]
    fps: Prop<f64>,

    running: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[thing_impl]
impl Camera {
    /// Starts the camera's thread, which feeds the stream.
    #[on_start]
    async fn start_camera(&self) -> anyhow::Result<()> {
        let preview = self.preview.clone();
        let fps = self.fps.subscribe();
        let running = Arc::clone(&self.running);
        running.store(true, Ordering::SeqCst);
        let thread = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                let mut n = 0;
                while running.load(Ordering::SeqCst) {
                    // `add_frame` never waits, however slow the clients.
                    if let Err(error) = render(n).map(|jpeg| preview.add_frame(jpeg)) {
                        eprintln!("the camera failed: {error}");
                    }
                    n += 1;
                    std::thread::sleep(Duration::from_secs_f64(1.0 / *fps.borrow()));
                }
            })?;
        *self.thread.lock().unwrap() = Some(thread);
        Ok(())
    }

    /// Stops the thread, and ends the clients' streams.
    #[on_stop]
    async fn stop_camera(&self) {
        self.running.store(false, Ordering::SeqCst);
        let thread = self.thread.lock().unwrap().take();
        if let Some(thread) = thread {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
        self.preview.stop();
    }

    /// The size of the next frame, in bytes: sharper images compress less,
    /// so autofocus routines maximise it.
    #[action]
    async fn sharpness(&self) -> Result<usize, ActionError> {
        Ok(self.preview.next_frame_size().await?)
    }

    /// Save the next frame.
    #[action]
    async fn snapshot(&self) -> Result<Blob<Jpeg>, ActionError> {
        Ok(Blob::from_bytes(self.preview.grab_frame().await?))
    }
}

/// Frame `n`: a gradient with a bar sweeping across it, and a frame counter
/// in binary along the top.
fn render(n: u64) -> anyhow::Result<Vec<u8>> {
    let (w, h) = (usize::from(WIDTH), usize::from(HEIGHT));
    let bar = (n as usize * 8) % w;
    let mut pixels = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let bit = x / 16;
            let pixel = if y < 12 && bit < 16 {
                if (n >> (15 - bit)) & 1 == 1 {
                    [255, 255, 255]
                } else {
                    [20, 20, 20]
                }
            } else if x.abs_diff(bar) < 6 {
                [255, 170, 0]
            } else {
                [(x * 90 / w) as u8, (y * 120 / h) as u8 + 40, 140]
            };
            pixels.extend(pixel);
        }
    }
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 80).encode(
        &pixels,
        WIDTH,
        HEIGHT,
        jpeg_encoder::ColorType::Rgb,
    )?;
    Ok(jpeg)
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
async fn invoke(client: &TestClient, action: &str) -> anyhow::Result<Value> {
    let created = client.post_json(&format!("/camera/{action}"), None).await;
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
        let address = listener.local_addr()?;
        println!("listening on http://{address}");
        println!("open http://{address}/camera/preview/viewer in a browser");
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: read three frames as a browser would.
    let client = TestClient::start(server()?).await?;
    let response = client.stream("/camera/preview").await;
    println!(
        "GET /camera/preview → {} {}",
        response.status(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
    );
    let mut body = response.into_body();
    for _ in 0..3 {
        let chunk = body
            .frame()
            .await
            .ok_or_else(|| anyhow::anyhow!("the stream ended"))??
            .into_data()
            .map_err(|_| anyhow::anyhow!("not data"))?;
        let header_end = chunk
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_default();
        println!(
            "  part: {:?} + {} bytes of JPEG",
            String::from_utf8_lossy(&chunk[..header_end]),
            chunk.len() - header_end - 6
        );
    }
    drop(body);

    let sharpness = invoke(&client, "sharpness").await?;
    println!("POST /camera/sharpness → {} bytes", sharpness["output"]);
    let snapshot = invoke(&client, "snapshot").await?;
    println!("POST /camera/snapshot → {}", snapshot["output"]["href"]);
    let viewer = client.get("/camera/preview/viewer").await;
    println!("GET /camera/preview/viewer → {}", viewer.text());
    let td = client.get("/camera/").await.json();
    println!("The TD's links: {}", td["links"]);
    client.stop().await;
    Ok(())
}
