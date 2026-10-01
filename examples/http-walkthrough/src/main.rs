//! Every route of the HTTP binding, over a real socket.
//!
//! ```text
//! cargo run -p http-walkthrough                   # serve on a free port and walk through it
//! cargo run -p http-walkthrough -- --serve        # serve on 127.0.0.1:5000 for walkthrough.sh/.ps1
//! ```
//!
//! The walkthrough sends the same requests as the curl scripts, with a
//! deliberately minimal HTTP/1.1 client, and prints each exchange as
//! `curl -i` would.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A dimmable lamp.
struct Lamp {
    brightness: Prop<u8>,
    is_on: Prop<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FadeInput {
    /// Target brightness.
    to: u8,
    /// Seconds per step.
    #[serde(default = "tenth")]
    step_seconds: f64,
}

fn tenth() -> f64 {
    0.1
}

impl Thing for Lamp {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Lamp")
            .description("A dimmable lamp.")
            .property(
                "brightness",
                DataProperty::new(|t: &Lamp| &t.brightness)
                    .doc("Brightness, 0 to 100.")
                    .unit("percent"),
            )
            .property(
                "is_on",
                DataProperty::new(|t: &Lamp| &t.is_on)
                    .read_only()
                    .doc("Whether the lamp is on."),
            )
            .action(
                "toggle",
                Action::new(|t: Arc<Lamp>, _: ActionCtx, _: NoInput| async move {
                    t.is_on.update(|on| *on = !*on)?;
                    Ok::<_, ActionError>(t.is_on.get())
                })
                .doc("Switch the lamp on or off.\n\nReturns whether it is now on."),
            )
            .action(
                "fade",
                Action::new(
                    |t: Arc<Lamp>, ctx: ActionCtx, input: FadeInput| async move {
                        while t.brightness.get() != input.to {
                            ctx.sleep(Duration::from_secs_f64(input.step_seconds))
                                .await?;
                            t.brightness
                                .update(|b| if *b < input.to { *b += 1 } else { *b -= 1 })?;
                        }
                        Ok::<_, ActionError>(t.brightness.get())
                    },
                )
                .doc("Fade to a brightness, one step at a time."),
            )
    }
}

fn server() -> anyhow::Result<ThingServer> {
    let lamp = Lamp {
        brightness: Prop::new(50).with_constraints(Constraints::new().le(100)),
        is_on: Prop::new(false),
    };
    Ok(ThingServer::builder().thing("lamp", lamp).build()?)
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
        let listener = TcpListener::bind("127.0.0.1:5000").await?;
        println!("listening on http://{}", listener.local_addr()?);
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server()?.serve_with(listener, async move {
        let _ = stopped.await;
    }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    walkthrough(addr).await?;
    let _ = stop.send(());
    serving.await??;
    Ok(())
}

/// The same requests as `walkthrough.sh` and `walkthrough.ps1`.
async fn walkthrough(addr: SocketAddr) -> anyhow::Result<()> {
    let origin = ("Origin", "http://example.com");
    section("Discovery: the Thing list and the Thing Description");
    http(addr, "GET", "/things/", None, &[]).await?;
    http(addr, "GET", "/lamp/", None, &[]).await?;

    section("Properties: read, write (with lax coercion), invalid values, reset");
    http(addr, "GET", "/lamp/brightness", None, &[]).await?;
    http(addr, "PUT", "/lamp/brightness", Some("\"80\""), &[]).await?;
    http(addr, "PUT", "/lamp/brightness", Some("150"), &[]).await?;
    http(addr, "PUT", "/lamp/is_on", Some("true"), &[]).await?;
    http(addr, "POST", "/lamp/brightness/reset", None, &[]).await?;

    section("Actions: invoke, poll, output, list");
    let invocation = http(addr, "POST", "/lamp/toggle", None, &[]).await?;
    let id = invocation["id"].as_str().unwrap_or_default().to_owned();
    tokio::time::sleep(Duration::from_millis(50)).await;
    http(addr, "GET", &format!("/action_invocations/{id}"), None, &[]).await?;
    http(
        addr,
        "GET",
        &format!("/action_invocations/{id}/output"),
        None,
        &[],
    )
    .await?;
    http(addr, "GET", "/lamp/toggle", None, &[]).await?;
    http(addr, "POST", "/lamp/fade", Some(r#"{"to": "high"}"#), &[]).await?;

    section("Cancelling a long invocation");
    let fade = http(addr, "POST", "/lamp/fade", Some(r#"{"to": 100}"#), &[]).await?;
    let id = fade["id"].as_str().unwrap_or_default().to_owned();
    tokio::time::sleep(Duration::from_millis(250)).await;
    http(
        addr,
        "DELETE",
        &format!("/action_invocations/{id}"),
        None,
        &[],
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    http(addr, "GET", &format!("/action_invocations/{id}"), None, &[]).await?;
    http(
        addr,
        "DELETE",
        &format!("/action_invocations/{id}"),
        None,
        &[],
    )
    .await?;

    section("Errors and routing: 404, 405, 307, 422");
    http(
        addr,
        "GET",
        "/action_invocations/00000000-0000-0000-0000-000000000000",
        None,
        &[],
    )
    .await?;
    http(addr, "GET", "/action_invocations/not-a-uuid", None, &[]).await?;
    http(addr, "DELETE", "/lamp/brightness", None, &[]).await?;
    http(addr, "GET", "/lamp", None, &[]).await?;

    section("CORS: a preflight, and a request with an Origin");
    http(
        addr,
        "OPTIONS",
        "/lamp/brightness",
        None,
        &[origin, ("Access-Control-Request-Method", "PUT")],
    )
    .await?;
    http(addr, "GET", "/lamp/is_on", None, &[origin]).await?;
    Ok(())
}

fn section(title: &str) {
    println!("\n## {title}\n");
}

/// Sends one request and prints it and the response, `curl -i` style.
/// Returns the JSON body (`null` if there is none).
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    request.push_str(body.unwrap_or_default());
    println!(
        "$ {method} {path}{}",
        body.map(|b| format!("  {b}")).unwrap_or_default()
    );

    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    for line in head
        .lines()
        .filter(|l| !l.starts_with("date:") && !l.starts_with("content-length:"))
    {
        println!("< {line}");
    }
    if !body.is_empty() {
        let shown: String = body.chars().take(300).collect();
        println!("{shown}{}", if body.len() > 300 { " …" } else { "" });
    }
    println!();
    Ok(serde_json::from_str(body).unwrap_or(Value::Null))
}
