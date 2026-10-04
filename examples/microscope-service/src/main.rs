//! The simulated microscope as a Windows service.
//!
//! ```text
//! microscope-service install [--port 5000]   register the service (administrator)
//! microscope-service uninstall               stop and remove it (administrator)
//! microscope-service run -c CONFIG [--port N] [--log FILE]
//!                                         what the Service Control Manager starts
//! microscope-service console -c CONFIG [--port N]
//!                                         the same server in a console, until Ctrl-C
//! microscope-service                      a demonstration of the stop path, then exit
//! ```
//!
//! A service stops the way a console server does: the Service Control
//! Manager's *stop* request completes the server's shutdown future, so
//! invocations are cancelled, the Things stop in reverse order, and their
//! devices close. While it does, the service reports *stop
//! pending* with a wait hint that covers the grace period.
//!
//! `install` writes the service's configuration next to the executable,
//! with absolute paths (a service starts in `C:\Windows\System32`), and the
//! service logs to `microscope-service.log` there. `service_test.ps1`
//! installs, starts, uses, stops and removes it.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use simulated_microscope::{CONFIG, registry};
use teta_wot::server::service::{self, InstallOptions, ServiceError};
use teta_wot::server::{cli, shutdown_signal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The service's name, as `sc` and `Start-Service` know it.
const NAME: &str = "teta-wot-microscope";

/// The grace period for requests and invocations when stopping.
const GRACE: Duration = Duration::from_secs(5);

/// What the Service Control Manager is told stopping may take: the grace
/// period, and time to stop the Things.
const STOP_HINT: Duration = Duration::from_secs(20);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => demonstrate(),
        Some("install") => install(&args[1..]),
        Some("uninstall") => service::uninstall(NAME)
            .map(|()| println!("removed the service `{NAME}`"))
            .map_err(Into::into),
        Some("run") => return run(&args[1..]),
        Some("console") => return console(&args[1..]),
        Some(other) => Err(anyhow::anyhow!(
            "unknown command `{other}`: use install, uninstall, run, console, or nothing"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The server's command line.
fn cli_args(args: &[String]) -> Vec<OsString> {
    std::iter::once(OsString::from("microscope-service"))
        .chain(args.iter().map(OsString::from))
        .collect()
}

/// What the Service Control Manager starts.
fn run(args: &[String]) -> ExitCode {
    // A service has no console: it logs to a file.
    let mut args = args.to_vec();
    if let Some(i) = args.iter().position(|a| a == "--log") {
        let file = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..(i + 2).min(args.len()));
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
        {
            Ok(file) => {
                let _ = tracing_subscriber::fmt()
                    .with_writer(std::sync::Mutex::new(file))
                    .with_ansi(false)
                    .try_init();
            }
            Err(error) => eprintln!("can't log to {file}: {error}"),
        }
    }
    let cli = cli_args(&args);
    tracing::info!("starting as the service `{NAME}`");
    // ANCHOR: run
    let result = service::run_as_service(NAME, STOP_HINT, move |_, stop| {
        Box::pin(async move {
            cli::serve_until(cli, &registry(), |b| b.shutdown_grace(GRACE), stop).await
        })
    });
    // ANCHOR_END: run
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(ServiceError::NotAService) => {
            eprintln!(
                "`run` is for the Service Control Manager: use `console` to run in a console"
            );
            ExitCode::from(2)
        }
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The same server in a console, until Ctrl-C.
fn console(args: &[String]) -> ExitCode {
    let cli = cli_args(args);
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(cli::serve_until(
            cli,
            &registry(),
            |b| b.shutdown_grace(GRACE),
            shutdown_signal(),
        )),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Registers the service, with its configuration next to the executable.
fn install(args: &[String]) -> anyhow::Result<()> {
    let port = match args.iter().position(|a| a == "--port") {
        Some(i) => args
            .get(i + 1)
            .context("--port needs a number")?
            .parse::<u16>()?,
        None => 5000,
    };
    let executable = std::env::current_exe()?;
    let folder = executable.parent().context("the executable's folder")?;
    let config = write_config(folder)?;
    let log = folder.join("microscope-service.log");
    service::install(&InstallOptions {
        name: NAME.into(),
        display_name: "teta-wot simulated microscope".into(),
        description: "A simulated stage, camera and autofocus, served as W3C Web of Things Things (the teta-wot microscope-service example).".into(),
        executable: executable.clone(),
        arguments: ["run", "-c"]
            .into_iter()
            .map(OsString::from)
            .chain([config.into_os_string()])
            .chain(["--port".into(), port.to_string().into(), "--log".into(), log.clone().into_os_string()])
            .collect(),
        automatic: false,
    })?;
    println!("installed the service `{NAME}`: {}", executable.display());
    println!("  start it with `Start-Service {NAME}`, and it serves http://127.0.0.1:{port}/");
    println!("  it logs to {}", log.display());
    Ok(())
}

/// The microscope's configuration, with an absolute settings folder next
/// to `folder`.
fn write_config(folder: &Path) -> anyhow::Result<PathBuf> {
    let mut config: Value = serde_json::from_str(CONFIG)?;
    config["settings_folder"] = json!(folder.join("settings").join("microscope-service"));
    let path = folder.join("microscope-service.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(path)
}

/// Without the Service Control Manager (and without an administrator): the
/// server as the service runs it, stopped through the same shutdown future
/// that the SCM's *stop* request completes.
fn demonstrate() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let folder = std::env::temp_dir().join(format!("wot-service-demo-{}", std::process::id()));
        std::fs::create_dir_all(&folder)?;
        let config = write_config(&folder)?;
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port();
        let at = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let cli = cli_args(&["-c".into(), config.display().to_string(), "--port".into(), port.to_string()]);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let serving = tokio::spawn(async move {
            let registry = registry();
            cli::serve_until(cli, &registry, |b| b.shutdown_grace(GRACE), async {
                let _ = stopped.await;
            })
            .await
        });

        let deadline = Instant::now() + Duration::from_secs(30);
        while get(at, "/stage/").await.is_err() {
            if Instant::now() > deadline {
                bail!("the server didn't start");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("serving the microscope on http://{at}/, as the service would");
        let invocation = post(at, "/stage/move_to", r#"{"x": 100000, "y": 0, "z": 0}"#).await?;
        println!("started a long move: {}", invocation.lines().next().unwrap_or_default());

        println!("stopping, as the Service Control Manager's stop request does");
        let asked = Instant::now();
        let _ = stop.send(());
        let code = tokio::time::timeout(GRACE + Duration::from_secs(10), serving).await??;
        println!("stopped in {:.1} s, with exit code {code:?}", asked.elapsed().as_secs_f64());
        let _ = std::fs::remove_dir_all(&folder);
        anyhow::ensure!(code == ExitCode::SUCCESS, "the server stopped cleanly");
        anyhow::ensure!(asked.elapsed() < GRACE, "the long move was cancelled, not waited for");
        println!("\nAs a service: `microscope-service install`, then `Start-Service {NAME}` (administrator).");
        Ok(())
    })
}

async fn get(at: SocketAddr, path: &str) -> anyhow::Result<String> {
    request(
        at,
        &format!("GET {path} HTTP/1.1\r\nHost: {at}\r\nConnection: close\r\n\r\n"),
    )
    .await
}

async fn post(at: SocketAddr, path: &str, body: &str) -> anyhow::Result<String> {
    request(
        at,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {at}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
    .await
}

async fn request(at: SocketAddr, request: &str) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect(at).await?;
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}
