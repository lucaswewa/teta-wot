//! Quickstart counter
//!
//! ```text
//! cargo run -p counter-server                     # a short in-process demo
//! cargo run -p counter-server -- --serve          # serve on http://127.0.0.1:5000
//! cargo run -p counter-server -- --serve --port 0 # any free port
//! ```


use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;
use tokio::net::TcpListener;

/// A test thing with a counter property and a couple of actions.
struct TestThing {
    counter: Prop<i64>,
}

impl Thing for TestThing {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("TestThing")
            .description("A test thing with a counter property and a couple of actions.")
            .property(
                "counter",
                DataProperty::new(|t: &TestThing| &t.counter).read_only().doc("A pointless counter."),
            )
            .action(
                "increment_counter",
                Action::new(|t: Arc<TestThing>, _: ActionCtx, _: NoInput| async move {
                    t.counter.update(|c| *c += 1)?;
                    Ok::<_, ActionError>(())
                })
                .doc(
                    "Increment the counter property.\n\nThis action doesn't do very much - all it does, in fact,\n\
                     is increment the counter (which may be read using the\n`counter` property).",
                ),
            )
            .action(
                "slowly_increase_counter",
                Action::new(|t: Arc<TestThing>, ctx: ActionCtx, _: NoInput| async move {
                    for _ in 0..60 {
                        ctx.sleep(Duration::from_secs(1)).await?;
                        t.counter.update(|c| *c += 1)?;
                    }
                    Ok::<_, ActionError>(())
                })
                .doc("Increment the counter slowly over a minute."),
            )
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing(
            "counter",
            TestThing {
                counter: Prop::new(0),
            },
        )
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
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        println!("listening on http://{}", listener.local_addr()?);
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: the same requests the Python client makes.
    let client = TestClient::start(server()?).await?;
    println!(
        "GET /counter/counter → {}",
        client.get("/counter/counter").await.text()
    );
    let invocation = client.post_json("/counter/increment_counter", None).await;
    println!(
        "POST /counter/increment_counter → {} {}",
        invocation.status,
        invocation.json()["status"]
    );
    let href = invocation.json()["id"]
        .as_str()
        .map(|id| format!("/action_invocations/{id}"))
        .unwrap_or_default();
    loop {
        let polled = client.get(&href).await.json();
        if !matches!(polled["status"].as_str(), Some("pending" | "running")) {
            println!("GET {href} → {}", polled["status"]);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let value = client.get("/counter/counter").await.json();
    println!("GET /counter/counter → {value}");
    anyhow::ensure!(value == 1, "the counter should be 1");
    client.stop().await;
    Ok(())
}
