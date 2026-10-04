//! Setting up `tracing` for a `teta-wot` process.

use tracing::Level;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::logs::InvocationLogLayer;

/// The level invocation logs capture: INFO, or DEBUG in debug mode .
pub fn capture_level(debug: bool) -> Level {
    if debug { Level::DEBUG } else { Level::INFO }
}

/// The invocation log layer, with its own level filter, ready to add to a
/// subscriber. Its filter is independent of the console's, so invocation
/// logs work whatever `RUST_LOG` says.
pub fn invocation_log_layer<S>(debug: bool) -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let level = capture_level(debug);
    InvocationLogLayer::new(level).with_filter(LevelFilter::from_level(level))
}

/// Installs a global subscriber: console output filtered by `RUST_LOG`
/// (default `info`, or `debug` in debug mode) plus the invocation log layer.
///
/// Returns an error if a global subscriber is already installed.
pub fn init(debug: bool) -> Result<(), tracing_subscriber::util::TryInitError> {
    let default = if debug { "debug" } else { "info" };
    let console = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(console))
        .with(invocation_log_layer(debug))
        .try_init()
}
