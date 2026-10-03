//! A Thing for Eclipse Thingweb node-wot to consume.
//!
//! The same Thing is served twice, in each wire profile, because node-wot
//! 0.9.2 needs something from each:
//!
//! - it reads an action's output from the `invokeaction` response, which
//!   only a synchronous action in the `wot` profile answers with (200 and
//!   the output);
//! - it observes with `EventSource.onmessage`, which receives only unnamed
//!   server-sent events: the `tetathing` profile's. The `wot` profile names
//!   each event after its property, as the WoT SSE Profile requires.
//!
//! `cargo run -p node-wot-consumer` checks, in-process, that both TDs have
//! the forms node-wot uses. With `--serve`, it serves both until Ctrl-C:
//! the `tetathing` profile on port 8000 (or `--port`) and the `wot` profile
//! on the next, printing each Thing's URL, for `consumer.mjs`.

use std::net::Ipv4Addr;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, ensure};
use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::server::{WireProfile, shutdown_signal};
use teta_wot::testing::TestClient;
use tokio::net::TcpListener;

/// A dimmable lamp.
#[derive(Thing)]
pub struct Lamp {
    /// The brightness, in percent.
    #[property(default = 50, ge = 0, le = 100)]
    level: Prop<i64>,
}

#[thing_impl]
impl Lamp {
    /// Double a number.
    #[action(synchronous)]
    async fn double(&self, n: i64) -> i64 {
        2 * n
    }

    /// Fade to a level, slowly.
    #[action]
    async fn fade(&self, level: i64) -> Result<i64, ActionError> {
        cancellable_sleep(Duration::from_millis(200)).await?;
        self.level.set(level)?;
        Ok(level)
    }
}

fn server(profile: WireProfile) -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .wire_profile(profile)
        .thing("lamp", Lamp::default())
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
    if args.iter().any(|a| a == "--serve") {
        let port: u16 = match args.iter().position(|a| a == "--port") {
            Some(i) => args.get(i + 1).context("--port needs a number")?.parse()?,
            None => 8000,
        };
        return serve(port).await;
    }
    check().await
}

/// Serves the Thing in both profiles until Ctrl-C.
async fn serve(port: u16) -> anyhow::Result<()> {
    let tetathing = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let next = if port == 0 { 0 } else { port + 1 };
    let wot = TcpListener::bind((Ipv4Addr::LOCALHOST, next)).await?;
    println!("tetathing http://{}/lamp/", tetathing.local_addr()?);
    println!("wot http://{}/lamp/", wot.local_addr()?);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let until_stopped = move || {
        let mut stopped = stopped.clone();
        async move {
            let _ = stopped.wait_for(|s| *s).await;
        }
    };
    let servers = tokio::spawn(async move {
        tokio::try_join!(
            server(WireProfile::TetaThing)?.serve_with(tetathing, until_stopped()),
            server(WireProfile::Wot)?.serve_with(wot, until_stopped()),
        )?;
        anyhow::Ok(())
    });
    shutdown_signal().await;
    let _ = stop.send(true);
    servers.await?
}

/// Checks, without Node, the TDs and answers that node-wot relies on.
async fn check() -> anyhow::Result<()> {
    for profile in [WireProfile::TetaThing, WireProfile::Wot] {
        let client = TestClient::start(server(profile)?).await?;
        let td = client.get("/lamp/").await.json();
        let forms = &td["properties"]["level"]["forms"];
        ensure!(forms[0]["op"] == json!(["readproperty", "writeproperty"]));
        ensure!(forms[1]["subprotocol"] == "sse", "an SSE observation form");
        let synchronous = td["actions"]["double"]["synchronous"] == true;
        let invoked = client
            .post_json("/lamp/double", Some(&json!({"n": 21})))
            .await;
        let mut events = client.events("/lamp/level").await;
        client.put_json("/lamp/level", &json!(30)).await;
        let named = events.next_message().await.context("an event")?.event;
        println!("{profile}:");
        println!("  double is synchronous: {synchronous}");
        println!(
            "  invoking it answers {} {}",
            invoked.status.as_u16(),
            invoked.text()
        );
        println!("  observation events are named: {}", named.is_some());
        match profile {
            // node-wot observes, but can't read an output.
            WireProfile::TetaThing => ensure!(!synchronous && named.is_none()),
            // node-wot reads the output, but can't observe.
            _ => ensure!(synchronous && invoked.json() == 42 && named.is_some()),
        }
        client.stop().await;
    }
    println!("\nRun `node consumer.mjs` against `--serve` to see node-wot use them.");
    Ok(())
}
