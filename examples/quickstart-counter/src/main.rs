//! Quickstart counter, written with the authoring macros.
//!
//! ```text
//! cargo run -p quickstart-counter              # a short in-process demo
//! cargo run -p quickstart-counter -- --serve   # serve on http://127.0.0.1:5000
//! ```
//!

use std::process::ExitCode;
use std::time::Duration;

use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;
use tokio::net::TcpListener;

/// A test thing with a counter property and a couple of actions.
#[derive(Thing)]
pub struct TestThing {
    /// A pointless counter.
    #[property(default = 0, readonly)]
    counter: Prop<i64>,
}

#[thing_impl]
impl TestThing {
    /// Increment the counter property.
    ///
    /// This action doesn't do very much - all it does, in fact,
    /// is increment the counter (which may be read using the
    /// `counter` property).
    #[action]
    async fn increment_counter(&self) -> Result<(), ActionError> {
        self.counter.update(|c| *c += 1)?;
        Ok(())
    }

    /// Increment the counter slowly over a minute.
    #[action]
    async fn slowly_increase_counter(&self, ctx: ActionCtx) -> Result<(), ActionError> {
        for _ in 0..60 {
            ctx.sleep(Duration::from_secs(1)).await?;
            self.counter.update(|c| *c += 1)?;
        }
        Ok(())
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("counter", TestThing::default())
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
    if std::env::args().nth(1).as_deref() == Some("--serve") {
        teta_wot::logging::init(false)?;
        let listener = TcpListener::bind(("127.0.0.1", 5000)).await?;
        println!("listening on http://{}", listener.local_addr()?);
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    let client = TestClient::start(server()?).await?;
    let td = client.get("/counter/").await.json();
    println!("The TD's affordances, from the struct and the impl block:");
    println!("  properties: {:?}", keys(&td["properties"]));
    println!("  actions:    {:?}", keys(&td["actions"]));
    println!(
        "  increment_counter: title {}, description {}",
        td["actions"]["increment_counter"]["title"],
        td["actions"]["increment_counter"]["description"]
    );

    // Over HTTP, as a client would.
    let invocation = client.post_json("/counter/increment_counter", None).await;
    let href = invocation
        .header("location")
        .unwrap_or_default()
        .replace("http://testserver", "");
    while matches!(
        client.get(&href).await.json()["status"].as_str(),
        Some("pending" | "running")
    ) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    println!(
        "\nPOST /counter/increment_counter, then GET /counter/counter → {}",
        client.get("/counter/counter").await.text()
    );

    // In-process, as another Thing would: the generated `TestThingActions`
    // trait has a typed method per action.
    let counter = client
        .runtime()
        .thing_ref::<TestThing>("counter")
        .expect("the counter");
    counter
        .increment_counter()
        .await
        .map_err(ActionError::into_anyhow)?;
    let value = ThingRef::thing(&counter).counter.get();
    println!("counter.increment_counter() in-process, then the counter is {value}");
    anyhow::ensure!(value == 2, "the counter should be 2");
    client.stop().await;
    Ok(())
}

fn keys(value: &serde_json::Value) -> Vec<&str> {
    value
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default()
}
