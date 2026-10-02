//! The command line of a Thing server:
//!
//! ```text
//! my-server -c config.json [--host 127.0.0.1] [--port 5000] [--fallback] [--debug]
//! my-server -j '{"things": {…}}'
//! ```
//!
//! An application calls [`serve_from_cli`] from its `main`, with the
//! registry of the Thing types its configuration files may name.
//!
//! Exit codes:
//!
//! | Code | When |
//! |---|---|
//! | 0 | The server stopped normally |
//! | 2 | The command line is wrong |
//! | 3 | The configuration is invalid, a Thing failed to start, or the fallback server was served |
//! | 1 | Anything else (no configuration, a missing file, a Thing that can't be built, a port in use) |

use std::ffi::OsString;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use teta_wot_http::{FallbackPage, fallback_router};
use tokio::net::TcpListener;

use crate::config::ServerConfig;
use crate::registry::ThingRegistry;
use crate::{ServeError, ThingServer, ThingServerBuilder, shutdown_signal};

/// The command-line options.
#[derive(Debug, Clone, Parser)]
#[command(about = "Serve Things over HTTP, from a configuration file.")]
pub struct CliArgs {
    /// Path to configuration file
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Configuration as JSON string
    #[arg(short, long)]
    pub json: Option<String>,
    /// Serve an error page instead of exiting, if we can't start.
    #[arg(long)]
    pub fallback: bool,
    /// Bind socket to this host
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Bind socket to this port. If 0, an available port will be picked.
    #[arg(long, default_value_t = 5000)]
    pub port: u16,
    /// Enable debug logging.
    #[arg(long)]
    pub debug: bool,
}

/// Why serving failed, and so the exit code.
#[derive(Debug)]
enum Failure {
    /// The configuration is invalid (exit code 3).
    Config(String),
    /// A Thing failed to start (exit code 3).
    Startup {
        thing: String,
        error: String,
        things: Vec<String>,
    },
    /// Anything else (exit code 1).
    Other(String),
}

impl Failure {
    fn message(&self) -> String {
        match self {
            Failure::Config(message) | Failure::Other(message) => message.clone(),
            Failure::Startup { thing, error, .. } => {
                format!("Failed to enter '{thing}' Thing: {error}")
            }
        }
    }
}

/// Parses the process's arguments and serves, returning the exit code.
pub async fn serve_from_cli(registry: &ThingRegistry) -> ExitCode {
    serve_from_args(std::env::args_os(), registry, |builder| builder).await
}

/// Like [`serve_from_cli`], with explicit arguments (the first is the
/// program name) and a chance to change the server builder, for example to
/// register services.
pub async fn serve_from_args<I, T>(
    args: I,
    registry: &ThingRegistry,
    customise: impl FnOnce(ThingServerBuilder) -> ThingServerBuilder,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    serve_until(args, registry, customise, shutdown_signal()).await
}

/// Like [`serve_from_args`], stopping when `shutdown` completes rather than
/// on Ctrl-C: for tests, examples and services.
pub async fn serve_until<I, T>(
    args: I,
    registry: &ThingRegistry,
    customise: impl FnOnce(ThingServerBuilder) -> ThingServerBuilder,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let _shutdown = tokio::spawn(async move {
        shutdown.await;
        let _ = stop.send(true);
    });
    let stop = Stop(stopped);
    let args = match CliArgs::try_parse_from(args) {
        Ok(args) => args,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let _ = teta_wot_core::logging::init(args.debug);
    let mut config_text = None;
    let result = run(&args, registry, customise, &mut config_text, &stop).await;
    let Err(failure) = result else {
        return ExitCode::SUCCESS;
    };

    if args.fallback {
        println!("Error: {}", failure.message());
        println!("Starting fallback server.");
        let page = FallbackPage {
            error_message: failure.message(),
            things: match &failure {
                Failure::Startup { things, .. } => things.clone(),
                _ => Vec::new(),
            },
            config: config_text.unwrap_or_default(),
            traceback: match &failure {
                Failure::Startup { error, .. } => error.clone(),
                other => other.message(),
            },
            logging: None,
        };
        if let Err(error) = serve_fallback(&args, page, &stop).await {
            eprintln!("Error: the fallback server couldn't start: {error}");
        }
        return ExitCode::from(3);
    }
    match failure {
        Failure::Config(message) => {
            println!("Error reading configuration:\n{message}");
            ExitCode::from(3)
        }
        startup @ Failure::Startup { .. } => {
            eprintln!("Error: {}", startup.message());
            ExitCode::from(3)
        }
        Failure::Other(message) => {
            eprintln!("Error: {message}");
            ExitCode::from(1)
        }
    }
}

async fn run(
    args: &CliArgs,
    registry: &ThingRegistry,
    customise: impl FnOnce(ThingServerBuilder) -> ThingServerBuilder,
    config_text: &mut Option<String>,
    stop: &Stop,
) -> Result<(), Failure> {
    let text = match (&args.config, &args.json) {
        (Some(_), Some(_)) => {
            return Err(Failure::Other(
                "Can't use both --config and --json simultaneously.".into(),
            ));
        }
        (Some(path), None) => std::fs::read_to_string(path).map_err(|error| {
            Failure::Other(format!(
                "Could not find configuration file {} ({error})",
                path.display()
            ))
        })?,
        (None, Some(json)) => json.clone(),
        (None, None) => {
            return Err(Failure::Other(
                "No configuration (or empty configuration) provided".into(),
            ));
        }
    };
    *config_text = Some(text.clone());
    let config = ServerConfig::from_json(&text).map_err(|e| Failure::Config(e.to_string()))?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        *config_text = serde_json::to_string_pretty(&value).ok();
    }
    let builder =
        ThingServer::from_config(&config, registry).map_err(|e| Failure::Config(e.to_string()))?;
    let server = customise(builder)
        .build()
        .map_err(|e| Failure::Other(e.to_string()))?;
    let things = server
        .runtime()
        .things()
        .map(|t| t.name().to_owned())
        .collect();

    let listener = bind(args).await.map_err(Failure::Other)?;
    match listener.local_addr() {
        Ok(addr) => println!("listening on http://{addr}"),
        Err(_) => println!("listening"),
    }
    match server.serve_with(listener, stop.wait()).await {
        Ok(()) => Ok(()),
        Err(ServeError::Startup(failure)) => Err(Failure::Startup {
            thing: failure.thing,
            error: format!("{:#}", failure.error),
            things,
        }),
        Err(ServeError::Io(error)) => Err(Failure::Other(error.to_string())),
    }
}

async fn bind(args: &CliArgs) -> Result<TcpListener, String> {
    let address = format!("{}:{}", args.host, args.port);
    let address: SocketAddr = match address.parse() {
        Ok(address) => address,
        Err(_) => tokio::net::lookup_host(&address)
            .await
            .map_err(|e| format!("can't resolve {address}: {e}"))?
            .next()
            .ok_or_else(|| format!("can't resolve {address}"))?,
    };
    TcpListener::bind(address)
        .await
        .map_err(|e| format!("can't listen on {address}: {e}"))
}

async fn serve_fallback(args: &CliArgs, page: FallbackPage, stop: &Stop) -> Result<(), String> {
    let listener = bind(args).await?;
    if let Ok(addr) = listener.local_addr() {
        println!("listening on http://{addr}");
    }
    axum::serve(listener, fallback_router(page))
        .with_graceful_shutdown(stop.wait())
        .await
        .map_err(|e| e.to_string())
}

/// The shutdown request, shared by the server and the fallback server.
struct Stop(tokio::sync::watch::Receiver<bool>);

impl Stop {
    fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut stopped = self.0.clone();
        async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }
    }
}
