//! An application binary: it registers its Thing types, and serves them
//! from configuration files.
//!
//! ```text
//! cargo run -p config-and-cli                                   # a demonstration, then exit
//! cargo run -p config-and-cli -- -c examples/config-and-cli/config.json
//! cargo run -p config-and-cli -- -c examples/config-and-cli/failing.json --fallback
//! ```

use std::ffi::OsString;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use teta_wot::prelude::*;
use teta_wot::server::{ThingRegistry, cli};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// A counter.
#[derive(Thing)]
#[thing(config = CounterConfig)]
pub struct Counter {
    /// The count.
    #[property(default = config.start, readonly)]
    count: Prop<i64>,
}

/// The counter's configuration: its `kwargs`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterConfig {
    #[serde(default)]
    start: i64,
}

#[thing_impl]
impl Counter {
    /// Add one.
    #[action]
    async fn increment(&self) -> Result<i64, ActionError> {
        self.count.update(|c| *c += 1)?;
        Ok(self.count.get())
    }
}

/// A stage whose hardware isn't connected: it can't start.
#[derive(Thing)]
pub struct DisconnectedStage;

#[thing_impl]
impl DisconnectedStage {
    #[on_start]
    async fn connect(&self) -> anyhow::Result<()> {
        anyhow::bail!("no stage on COM3")
    }
}

/// The Thing types this application serves, by the names its configuration
/// files use.
fn registry() -> ThingRegistry {
    ThingRegistry::new()
        .register::<Counter>("my_lab.counters:Counter")
        .register::<DisconnectedStage>("my_lab.stages:Stage")
}

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        // The real command line.
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

/// Runs the command line in-process with a configuration, calls `visit`
/// while it serves, stops it and returns its exit code.
async fn run_cli<F>(extra: &[&str], config: &Value, visit: F) -> anyhow::Result<ExitCode>
where
    F: AsyncFnOnce(u16) -> anyhow::Result<()>,
{
    let port = TcpListener::bind("127.0.0.1:0").await?.local_addr()?.port();
    let mut args: Vec<OsString> = ["config-and-cli", "-j"].map(OsString::from).to_vec();
    args.push(config.to_string().into());
    args.extend(["--port".into(), port.to_string().into()]);
    args.extend(extra.iter().map(OsString::from));
    let (stop, stopped) = oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        cli::serve_until(args, &registry(), |b| b, async {
            let _ = stopped.await;
        })
        .await
    });
    // Wait until something answers, unless the command line has ended.
    for _ in 0..200 {
        if serving.is_finished() || TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if !serving.is_finished() {
        visit(port).await?;
    }
    let _ = stop.send(());
    Ok(serving.await?)
}

/// A minimal HTTP/1.1 GET: the status code and the body.
async fn get(port: u16, path: &str) -> anyhow::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    Ok((status, body))
}

async fn demo() -> anyhow::Result<()> {
    let settings = tempfile::tempdir()?;
    let folder = settings.path().to_string_lossy().to_string();

    println!("1. A configuration:");
    let config = json!({
        "things": {
            "counter": {"class": "my_lab.counters:Counter", "kwargs": {"start": 41}},
            "spare": "my_lab.counters:Counter",
        },
        "api_prefix": "/api",
        "settings_folder": folder,
    });
    println!("   {config}");
    let code = run_cli(&[], &config, async |port| {
        let (_, things) = get(port, "/api/things/").await?;
        println!("   GET /api/things/ → {things}");
        let (_, count) = get(port, "/api/counter/count").await?;
        println!("   GET /api/counter/count → {count}");
        anyhow::ensure!(count == "41");
        Ok(())
    })
    .await?;
    println!("   exit code: {code:?}\n");
    anyhow::ensure!(code == ExitCode::SUCCESS);

    println!("2. A Thing that can't start, with --fallback:");
    let failing = json!({
        "things": {"counter": "my_lab.counters:Counter", "stage": "my_lab.stages:Stage"},
        "settings_folder": folder,
    });
    let code = run_cli(&["--fallback"], &failing, async |port| {
        let (_, identified) = get(port, "/fallback").await?;
        let (status, _) = get(port, "/").await?;
        let (redirect, _) = get(port, "/stage/").await?;
        println!("   GET /fallback → {identified}; GET / → {status}; GET /stage/ → {redirect}");
        anyhow::ensure!(identified == "true" && status == 500 && redirect == 307);
        Ok(())
    })
    .await?;
    println!("   exit code: {code:?}\n");
    anyhow::ensure!(code == ExitCode::from(3));

    println!("3. An unknown class is a configuration error:");
    let unknown = json!({"things": {"x": "my_lab.missing:Thing"}, "settings_folder": folder});
    let code = run_cli(&[], &unknown, async |_| Ok(())).await?;
    println!("   exit code: {}", number(code));
    anyhow::ensure!(code == ExitCode::from(3));
    Ok(())
}

/// An exit code as a number, for the ones this demonstration expects.
fn number(code: ExitCode) -> &'static str {
    if code == ExitCode::SUCCESS {
        "0"
    } else if code == ExitCode::from(3) {
        "3"
    } else {
        "another code"
    }
}
