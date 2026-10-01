//! A server mounted under `/api/v1`.
//!
//! Every route moves under the prefix, and so does every URL the server
//! writes: form `href`s in the TD, the Thing list, invocation `href`s and
//! links, and the `Location` header. The TD's `base` stays the server's root.
//! The example also shows that URLs are built from the request: the same
//! server answers with the host name the client used.

use std::process::ExitCode;
use std::sync::Arc;

use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

struct Counter {
    count: Prop<i64>,
}

impl Thing for Counter {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Counter")
            .property(
                "count",
                DataProperty::new(|t: &Counter| &t.count).read_only(),
            )
            .action(
                "increment",
                Action::new(|t: Arc<Counter>, _: ActionCtx, _: NoInput| async move {
                    t.count.update(|c| *c += 1)?;
                    Ok::<_, ActionError>(t.count.get())
                }),
            )
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
    let server = ThingServer::builder()
        .api_prefix("/api/v1")
        .thing(
            "counter",
            Counter {
                count: Prop::new(0),
            },
        )
        .build()?;
    let client = TestClient::start(server)
        .await?
        .with_host("lab-pc.local:8000");

    let things = client.get("/api/v1/things/").await.json();
    println!("GET /api/v1/things/\n  {things}");
    anyhow::ensure!(things["counter"] == "http://lab-pc.local:8000/api/v1/counter/");

    let td = client.get("/api/v1/counter/").await.json();
    println!("GET /api/v1/counter/");
    println!("  base: {}", td["base"]);
    println!("  id:   {}", td["id"]);
    println!(
        "  count form:     {}",
        td["properties"]["count"]["forms"][0]["href"]
    );
    println!(
        "  increment form: {}",
        td["actions"]["increment"]["forms"][0]["href"]
    );
    anyhow::ensure!(td["properties"]["count"]["forms"][0]["href"] == "/api/v1/counter/count");

    let invocation = client.post_json("/api/v1/counter/increment", None).await;
    let body = invocation.json();
    println!("POST /api/v1/counter/increment → {}", invocation.status);
    println!("  action:   {}", body["action"]);
    println!("  href:     {}", body["href"]);
    println!(
        "  Location: {}",
        invocation.header("location").unwrap_or("-")
    );
    anyhow::ensure!(body["action"] == "/api/v1/counter/increment");

    let unprefixed = client.get("/counter/").await;
    println!(
        "GET /counter/ → {} {}",
        unprefixed.status,
        unprefixed.text()
    );
    anyhow::ensure!(unprefixed.status == 404);
    client.stop().await;
    Ok(())
}
