//! The interactive API docs with their assets compiled in: with the
//! `docs-offline` feature, Swagger UI (`/docs`) and ReDoc (`/redoc`) load
//! nothing from the internet, so they work on a lab network without it.
//!
//! ```text
//! cargo run -p docs-offline                     # an in-process check
//! cargo run -p docs-offline -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p docs-offline -- --serve --port 0 # any free port
//! ```
//!
//! While it serves, open `http://127.0.0.1:5000/docs`.

use std::process::ExitCode;

use teta_wot::prelude::*;
use teta_wot::server::{DOCS_OFFLINE, REDOC_VERSION, SWAGGER_UI_VERSION, shutdown_signal};
use teta_wot::testing::TestClient;

/// A dimmable lamp.
#[derive(Thing)]
pub struct Lamp {
    /// The brightness, in percent.
    #[property(default = 50, ge = 0, le = 100)]
    brightness: Prop<u8>,
}

#[thing_impl]
impl Lamp {
    /// Switch the lamp off.
    #[action]
    async fn switch_off(&self) -> Result<u8, ActionError> {
        self.brightness.set(0)?;
        Ok(0)
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("lamp", Lamp::default())
        .api_info("Lamp", "1.0.0")
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
        println!("docs at http://{address}/docs and http://{address}/redoc");
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process check: the pages refer only to the server.
    anyhow::ensure!(DOCS_OFFLINE, "built without the docs-offline feature");
    println!("Swagger UI {SWAGGER_UI_VERSION} and ReDoc {REDOC_VERSION}, compiled in");
    let client = TestClient::start(server()?).await?;
    for page in ["/docs", "/redoc", "/docs/oauth2-redirect"] {
        let html = client.get(page).await.text();
        let external: Vec<&str> = html
            .split(['"', '\''])
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
            .collect();
        println!(
            "GET {page} → {} bytes, external URLs: {external:?}",
            html.len()
        );
        anyhow::ensure!(external.is_empty(), "{page} loads something from outside");
    }
    for asset in [
        "/docs/swagger-ui-bundle.js",
        "/docs/swagger-ui.css",
        "/docs/favicon-32x32.png",
        "/redoc/redoc.standalone.js",
    ] {
        let response = client.get(asset).await;
        println!(
            "GET {asset} → {} {}, {} bytes",
            response.status,
            response.header("content-type").unwrap_or("-"),
            response.body.len()
        );
        anyhow::ensure!(response.status == 200);
    }
    client.stop().await;
    Ok(())
}
