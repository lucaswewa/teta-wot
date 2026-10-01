//! Graceful shutdown.
//!
//! Two Things, each with a device. One runs a long action. When the server is
//! asked to stop, it:
//!
//! 1. stops accepting connections, and lets requests in progress finish;
//! 2. cancels unfinished invocations and waits (up to the grace period) for
//!    them to end;
//! 3. stops the Things in reverse order, each after its devices' users are
//!    done and before its devices close.
//!
//! ```text
//! cargo run -p graceful-shutdown              # stops itself after a moment
//! cargo run -p graceful-shutdown -- --wait    # waits for Ctrl-C, Ctrl-Break or closing the console
//! ```

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use tokio::net::TcpListener;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn say(line: &str) {
    let elapsed = START.get_or_init(Instant::now).elapsed();
    println!("[{:>5} ms] {line}", elapsed.as_millis());
}

/// A shutter driver: closing it is what must not be skipped.
struct Shutter {
    name: String,
}

impl Driver for Shutter {
    fn open(&mut self) -> anyhow::Result<()> {
        say(&format!(
            "{}: shutter opened (on {:?})",
            self.name,
            std::thread::current().name().unwrap_or("?")
        ));
        Ok(())
    }

    fn close(&mut self) -> anyhow::Result<()> {
        say(&format!("{}: shutter closed", self.name));
        Ok(())
    }
}

struct Camera {
    name: String,
    shutter: Device<Shutter>,
}

impl Camera {
    fn new(name: &str) -> Self {
        let driver_name = name.to_owned();
        Self {
            name: name.to_owned(),
            shutter: Device::new(move || {
                Ok(Shutter {
                    name: driver_name.clone(),
                })
            }),
        }
    }
}

impl Thing for Camera {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Camera").device("shutter", |t: &Camera| &t.shutter).action(
            "time_lapse",
            Action::new(|t: Arc<Camera>, ctx: ActionCtx, _: NoInput| async move {
                let mut frames = 0;
                loop {
                    if ctx.sleep(Duration::from_millis(100)).await.is_err() {
                        say(&format!("{}: time lapse cancelled after {frames} frames; finishing the file", t.name));
                        return Err::<(), ActionError>(Cancelled.into());
                    }
                    frames += 1;
                }
            }),
        )
    }

    async fn start(self: Arc<Self>, ctx: ThingCtx) -> anyhow::Result<()> {
        say(&format!("{}: started", ctx.name()));
        Ok(())
    }

    async fn stop(self: Arc<Self>, ctx: ThingCtx) {
        say(&format!("{}: stopping", ctx.name()));
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
    let wait_for_signal = std::env::args().nth(1).as_deref() == Some("--wait");
    say("building the server");
    let server = ThingServer::builder()
        .thing("left", Camera::new("left"))
        .thing("right", Camera::new("right"))
        .shutdown_grace(Duration::from_secs(3))
        .build()?;
    let runtime = Arc::clone(server.runtime());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    say(&format!("serving on http://{}", listener.local_addr()?));

    let shutdown = async move {
        if wait_for_signal {
            say("press Ctrl-C (or Ctrl-Break, or close the console) to stop");
            shutdown_signal().await;
        } else {
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
        say("shutdown requested");
    };
    let serving = tokio::spawn(server.serve_with(listener, shutdown));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let invocation = runtime
        .thing("left")
        .expect("added")
        .action("time_lapse")
        .expect("defined")
        .invoke(Value::Null)?;
    say("left: time lapse started");

    serving.await??;
    say(&format!(
        "left's time lapse ended as `{}`",
        invocation.status()
    ));
    anyhow::ensure!(
        invocation.status() == InvocationStatus::Cancelled,
        "the invocation should be cancelled"
    );
    Ok(())
}
