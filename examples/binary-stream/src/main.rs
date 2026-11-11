//! Binary streams: a WebSocket of binary messages for each client, of the
//! size and at the rate each client asks for. A client that falls behind
//! gets the newest message, and never holds up the camera or the others.
//!
//! A simulated camera streams frames. Four clients ask for 1 MB frames at
//! 60 Hz (the benchmark), and a fifth reads one frame every 200 ms. After a
//! few seconds (`--seconds N` to change), it reports what each client
//! received, and the server reports what it sent and dropped.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::Deserialize;
use teta_wot::http::Subscriber;
use teta_wot::prelude::*;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// What a client asks for.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
struct FrameParams {
    /// The size of each frame, in bytes.
    size: usize,
    /// Frames per second.
    rate: f64,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            size: 1 << 20,
            rate: 60.0,
        }
    }
}

/// A simulated camera.
#[derive(Thing)]
pub struct Camera {}

#[thing_impl]
impl Camera {
    /// Frames, of the size and at the rate each client asks for.
    #[binary_stream]
    async fn frames(&self, mut subscriber: Subscriber<FrameParams>) {
        'parameters: loop {
            let FrameParams { size, rate } = subscriber.params();
            let frame = Bytes::from(vec![0x5a; size]);
            let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / rate.max(0.1)));
            loop {
                tokio::select! {
                    changed = subscriber.changed() => {
                        if changed { continue 'parameters } else { break 'parameters }
                    }
                    _ = tick.tick() => {
                        subscriber.send(frame.clone());
                    }
                }
            }
        }
        println!(
            "  server: a client left after {} frames sent, {} dropped",
            subscriber.sent(),
            subscriber.dropped()
        );
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
    let seconds: f64 = match std::env::args().skip_while(|a| a != "--seconds").nth(1) {
        Some(seconds) => seconds.parse()?,
        None => 3.0,
    };
    let server = ThingServer::builder()
        .thing("camera", Camera::default())
        .build()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/camera/frames", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_with(listener, async move {
        let _ = stopped.await;
    }));

    let run_for = Duration::from_secs_f64(seconds);
    let mut clients = Vec::new();
    for client in 1..=5 {
        let slow = client == 5;
        let url = url.clone();
        clients.push(tokio::spawn(async move {
            receive(&url, run_for, slow)
                .await
                .map(|got| (client, slow, got))
        }));
    }
    println!("5 clients, for {seconds} s:");
    let mut fast_frames = Vec::new();
    for client in clients {
        let (client, slow, (frames, bytes)) = client.await??;
        let (fps, mb_s) = (frames as f64 / seconds, bytes as f64 / seconds / 1e6);
        let kind = if slow { "slow" } else { "fast" };
        println!("  client {client} ({kind}): {frames} frames, {fps:.1} frames/s, {mb_s:.1} MB/s");
        if !slow {
            fast_frames.push(fps);
        }
    }
    // Let the server's producers report, then stop it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = stop.send(());
    serving.await??;
    let slowest = fast_frames.iter().copied().fold(f64::INFINITY, f64::min);
    anyhow::ensure!(
        slowest > 30.0,
        "a fast client got only {slowest:.1} frames/s"
    );
    Ok(())
}

/// Receives frames for `run_for`, and returns how many and how many bytes.
/// The slow client checks that bad parameters are refused, then reads a
/// frame every 200 ms.
async fn receive(url: &str, run_for: Duration, slow: bool) -> anyhow::Result<(u64, u64)> {
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await?;
    if slow {
        socket.send(Message::text(r#"{"rate": "fast"}"#)).await?;
    }
    let (mut frames, mut bytes) = (0, 0);
    let deadline = Instant::now() + run_for;
    // `timeout_at` returns a message that is already waiting even after the
    // deadline, and a slow client always has one waiting: check the time too.
    while Instant::now() < deadline {
        let next = tokio::time::timeout_at(deadline.into(), socket.next()).await;
        let Ok(Some(message)) = next else { break };
        match message? {
            Message::Binary(frame) => {
                frames += 1;
                bytes += frame.len() as u64;
                if slow {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
            Message::Text(reply) => println!("  client 5 sent bad parameters: {reply}"),
            _ => {}
        }
    }
    // The server may already have gone: that is fine.
    let _ = socket.close(None).await;
    Ok((frames, bytes))
}
