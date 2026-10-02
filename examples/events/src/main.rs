//! Events: a laser's safety interlock trips on
//! its device thread, which emits an event straight from synchronous driver
//! code. Clients receive it over server-sent events and over the WebSocket;
//! Rust code can subscribe too.
//!
//! ```text
//! cargo run -p events                      # an in-process demo
//! cargo run -p events -- --serve           # serve on http://127.0.0.1:5000
//! cargo run -p events -- --serve --port 0  # any free port
//! ```
//!
//! While it serves:
//!
//! ```text
//! curl -N http://127.0.0.1:5000/laser/tripped              # waits for events
//! curl -X POST http://127.0.0.1:5000/laser/open_door       # in another terminal
//! ```

use std::process::ExitCode;

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use teta_wot::prelude::*;
use teta_wot::server::shutdown_signal;
use teta_wot::testing::TestClient;

/// Why the interlock tripped.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Trip {
    /// The input that opened the interlock circuit.
    input: String,
    /// The emission's state before it was cut.
    was_emitting: bool,
}

/// A simulated interlock circuit, read and driven on the device's thread.
#[derive(Default)]
pub struct Interlock {
    door_open: bool,
    emitting: bool,
    /// Where trips are reported: the Thing's event, given at start.
    on_trip: Option<Event<Trip>>,
}

impl Driver for Interlock {}

impl Interlock {
    fn set_door(&mut self, open: bool) {
        if open && !self.door_open {
            let trip = Trip {
                input: "enclosure door".to_owned(),
                was_emitting: self.emitting,
            };
            self.emitting = false;
            println!(
                "[{}] the interlock tripped: {trip:?}",
                std::thread::current().name().unwrap_or("?")
            );
            // Synchronous code, on the device thread: `emit` never waits.
            if let Some(event) = &self.on_trip {
                event.emit(trip);
            }
        }
        self.door_open = open;
    }
}

/// A laser in an enclosure with a safety interlock.
#[derive(Thing)]
pub struct Laser {
    #[device]
    interlock: Device<Interlock>,

    /// The interlock tripped, and emission stopped.
    ///
    /// Sent when the enclosure is opened while the interlock is armed.
    #[event]
    tripped: Event<Trip>,
}

#[thing_impl]
impl Laser {
    /// Gives the driver the event to emit (an `Event` is a cheap handle).
    #[on_start]
    async fn connect(&self) -> anyhow::Result<()> {
        let tripped = self.tripped.clone();
        self.interlock
            .call(move |interlock, _| interlock.on_trip = Some(tripped))
            .await?;
        Ok(())
    }

    /// Whether the laser is emitting.
    #[property]
    async fn emitting(&self) -> Result<bool, PropertyError> {
        self.interlock
            .call(|interlock, _| interlock.emitting)
            .await
            .map_err(PropertyError::failed)
    }

    /// Start emitting, if the door is closed.
    #[action]
    async fn start_emission(&self) -> Result<bool, ActionError> {
        let started = self
            .interlock
            .call(|interlock, _| {
                interlock.emitting = !interlock.door_open;
                interlock.emitting
            })
            .await?;
        Ok(started)
    }

    /// Open the enclosure door (in the simulation), which trips the interlock.
    #[action]
    async fn open_door(&self) -> Result<(), ActionError> {
        self.interlock
            .call(|interlock, _| interlock.set_door(true))
            .await?;
        Ok(())
    }

    /// Close the enclosure door.
    #[action]
    async fn close_door(&self) -> Result<(), ActionError> {
        self.interlock
            .call(|interlock, _| interlock.set_door(false))
            .await?;
        Ok(())
    }
}

fn server() -> anyhow::Result<ThingServer> {
    Ok(ThingServer::builder()
        .thing("laser", Laser::default())
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
        println!("  curl -N http://{address}/laser/tripped");
        println!("  curl -X POST http://{address}/laser/open_door");
        server()?.serve_with(listener, shutdown_signal()).await?;
        return Ok(());
    }

    // The in-process demo: three subscribers, one trip.
    let client = TestClient::start(server()?).await?;
    let laser = client
        .runtime()
        .thing_ref::<Laser>("laser")
        .expect("the laser");
    let mut in_rust = ThingRef::thing(&laser).tripped.subscribe();

    let mut sse = client.events("/laser/tripped").await;
    println!(
        "GET /laser/tripped → {} {}",
        sse.status,
        sse.header("content-type").unwrap_or("-")
    );

    let mut ws = client.websocket("/laser/ws").await;
    ws.send_json(&json!({"messageType": "addEventSubscription", "data": {"tripped": true}}))
        .await;
    // Refused (no such event); as messages are handled in order, the answer
    // also means the subscription above is in place.
    ws.send_json(&json!({"messageType": "addEventSubscription", "data": {"exploded": true}}))
        .await;
    println!("WebSocket ← {}", ws.receive_json().await);

    // In-process calls wait for the action; then the trip, over HTTP.
    laser
        .start_emission()
        .await
        .map_err(ActionError::into_anyhow)?;
    client.post_json("/laser/open_door", None).await;

    let data = sse.next().await;
    println!("SSE data: {}", data.clone().unwrap_or_default());
    anyhow::ensure!(data == Some(json!({"input": "enclosure door", "was_emitting": true})));
    let message = ws.receive_json().await;
    println!("WebSocket ← {message}");
    anyhow::ensure!(message["data"]["tripped"]["data"] == data.unwrap_or_default());
    println!("Rust subscriber: {:?}", in_rust.recv().await?);

    let td = client.get("/laser/").await.json();
    println!(
        "The TD's event: {}",
        serde_json::to_string_pretty(&td["events"])?
    );
    ws.close().await;
    client.stop().await;
    Ok(())
}
