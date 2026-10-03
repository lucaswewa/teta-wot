//! Discovery (W3C WoT Discovery): a server that can be found.
//!
//! The server advertises itself on the local network with DNS-SD over
//! mDNS, as a Thing Description Directory (`_directory._sub._wot._tcp`),
//! with the path of its TD in the TXT record. A client that knows nothing
//! about it browses for directories, reads the TD at `/.well-known/wot`,
//! and follows the directory's `things` form to every Thing's TD.
//!
//! `cargo run -p discovery` does both in one process, and exits. With
//! `--serve`, it only serves (on every interface, port 8000 unless
//! `--port` says otherwise) until Ctrl-C, for `browse.py` or any DNS-SD
//! browser on the network.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde_json::Value;
use teta_wot::prelude::*;
use teta_wot::server::{DIRECTORY_SERVICE_TYPE, shutdown_signal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A lamp, to be found.
#[derive(Thing)]
pub struct Lamp {
    /// The brightness, in percent.
    #[property(default = 50, ge = 0, le = 100)]
    level: Prop<i64>,
}

/// A thermometer, to be found too.
#[derive(Thing)]
pub struct Thermometer {
    /// The temperature, in degrees Celsius.
    #[property(default = 21.5, readonly)]
    temperature: Prop<f64>,
}

/// The DNS-SD instance name: the server ID.
const INSTANCE: &str = "wot-rs discovery example";

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
    let serve = args.iter().any(|a| a == "--serve");
    let port = match args.iter().position(|a| a == "--port") {
        Some(i) => args.get(i + 1).context("--port needs a number")?.parse()?,
        None if serve => 8000,
        None => 0,
    };

    // Every interface, since the advertisement names the computer's
    // addresses (Windows may ask whether to allow this).
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let port = listener.local_addr()?.port();
    let server = ThingServer::builder()
        .server_id(INSTANCE)
        .mdns(true)
        .thing("lamp", Lamp::default())
        .thing("thermometer", Thermometer::default())
        .build()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve_with(listener, async {
        let _ = stopped.await;
    }));

    if serve {
        println!("serving on port {port}, advertised as `{INSTANCE}`; Ctrl-C to stop");
        shutdown_signal().await;
    } else {
        let found = browse(port).await;
        let _ = stop.send(());
        serving.await??;
        return found;
    }
    let _ = stop.send(());
    serving.await??;
    Ok(())
}

/// Finds the server as a client would: by browsing for TD Directories.
async fn browse(port: u16) -> anyhow::Result<()> {
    let daemon = ServiceDaemon::new()?;
    let events = daemon.browse(DIRECTORY_SERVICE_TYPE)?;
    println!("browsing for {DIRECTORY_SERVICE_TYPE}");
    let service = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(event) = events.recv_async().await {
            if let ServiceEvent::ServiceResolved(service) = event {
                println!("  found `{}` at port {}", service.fullname, service.port);
                // Other directories on the network are someone else's.
                if service.port == port {
                    return Some(service);
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .context("the advertisement wasn't found within 20 s: is multicast allowed on this network?")?;
    let _ = daemon.shutdown();

    let txt = |key: &str| {
        service
            .get_property_val_str(key)
            .unwrap_or_default()
            .to_owned()
    };
    println!(
        "  TXT: td={} type={} scheme={}",
        txt("td"),
        txt("type"),
        txt("scheme")
    );
    ensure!(txt("type") == "Directory" && txt("scheme") == "http");

    // The advertised addresses are the computer's; the TD is at `td`.
    let mut addresses: Vec<IpAddr> = service
        .get_addresses_v4()
        .into_iter()
        .map(IpAddr::V4)
        .collect();
    addresses.sort();
    println!("  addresses: {addresses:?}");
    let mut directory = None;
    for address in addresses {
        let at = SocketAddr::new(address, service.port);
        if let Ok(td) = get_json(at, &txt("td")).await {
            directory = Some((at, td));
            break;
        }
    }
    let (at, directory) = directory.context("no advertised address answered")?;
    println!("\nGET http://{at}{}", txt("td"));
    println!("  {}: {}", directory["@type"], directory["title"]);

    // The directory's `things` form lists every TD.
    let base = directory["base"]
        .as_str()
        .context("the directory has a base")?;
    let href = directory["properties"]["things"]["forms"][0]["href"]
        .as_str()
        .context("the directory has a `things` form")?;
    let path = format!(
        "{}{}",
        base.split_once(&at.to_string())
            .map_or("/", |(_, path)| path),
        href.split('{').next().unwrap_or(href)
    );
    let tds = get_json(at, &path).await?;
    println!("GET http://{at}{path}");
    let tds = tds.as_array().context("the listing is an array")?;
    for td in tds {
        println!("  {} ({}): {}", td["title"], td["id"], td["base"]);
    }
    ensure!(tds.len() == 2, "both Things are listed");
    println!("\nFound the server and both its Things.");
    Ok(())
}

/// `GET path` as JSON, with a minimal HTTP/1.1 client.
async fn get_json(at: SocketAddr, path: &str) -> anyhow::Result<Value> {
    let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(at)).await??;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {at}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response = String::from_utf8(response)?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .context("an HTTP response")?;
    let status = head.split_whitespace().nth(1).unwrap_or_default();
    if status != "200" {
        bail!("GET {path}: {status}");
    }
    Ok(serde_json::from_str(body)?)
}
