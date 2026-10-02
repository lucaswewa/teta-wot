//! Thing slots: an autofocus that uses a
//! stage, a camera through an interface, an optional lamp, and every stage
//! on the server. Two configuration files connect it differently:
//! the second swaps the simulated camera for a "real" one, and adds a lamp.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use teta_wot::prelude::*;
use teta_wot::server::{ServerConfig, ThingRegistry};
use teta_wot::testing::TestClient;

/// What an autofocus needs from a camera.
#[teta_wot::interface]
pub trait CameraApi: Send + Sync {
    /// How sharp the image is with the focus at `z`.
    fn sharpness(&self, z: i64) -> BoxFuture<'_, Result<f64, ActionError>>;
}

/// A motorised focus stage.
#[derive(Thing)]
pub struct Stage {
    /// The focus position, in steps.
    #[property(default = 0, readonly)]
    z: Prop<i64>,
}

#[thing_impl]
impl Stage {
    /// Move the focus.
    #[action]
    async fn move_to(&self, z: i64) -> Result<i64, ActionError> {
        self.z.set(z)?;
        Ok(z)
    }
}

/// A simulated camera, in focus at 3.
#[derive(Thing)]
#[thing(interfaces(CameraApi))]
pub struct SimCamera;

#[thing_impl]
impl SimCamera {
    /// Simulate the sharpness of an image.
    #[action]
    async fn simulate(&self, z: i64) -> f64 {
        1.0 / (1.0 + ((z - 3) * (z - 3)) as f64)
    }
}

impl CameraApi for ThingRef<SimCamera> {
    fn sharpness(&self, z: i64) -> BoxFuture<'_, Result<f64, ActionError>> {
        self.simulate(z)
    }
}

/// The "real" camera: another simulation here, in focus at -2, standing in
/// for hardware.
#[derive(Thing)]
#[thing(interfaces(CameraApi))]
pub struct RealCamera;

#[thing_impl]
impl RealCamera {
    /// Measure the sharpness of an image.
    #[action]
    async fn measure(&self, z: i64) -> f64 {
        1.0 / (1.0 + ((z + 2) * (z + 2)) as f64)
    }
}

impl CameraApi for ThingRef<RealCamera> {
    fn sharpness(&self, z: i64) -> BoxFuture<'_, Result<f64, ActionError>> {
        self.measure(z)
    }
}

/// An LED lamp.
#[derive(Thing)]
pub struct Lamp {
    on: AtomicBool,
}

#[thing_impl]
impl Lamp {
    /// Switch the lamp on.
    #[action]
    async fn switch_on(&self) {
        self.on.store(true, Ordering::SeqCst);
    }
}

/// Focuses the camera by moving the stage.
#[derive(Thing)]
pub struct Autofocus {
    /// The stage: exactly one, found by type.
    #[slot]
    stage: Slot<Stage>,
    /// The camera: any Thing providing `CameraApi`, by default the one named
    /// `camera`.
    #[slot(default = "camera")]
    camera: Slot<dyn CameraApi>,
    /// A lamp, if there is one.
    #[slot]
    lamp: OptSlot<Lamp>,
    /// Every camera on the server, by name.
    #[slot]
    cameras: SlotMap<dyn CameraApi>,
}

#[thing_impl]
impl Autofocus {
    /// Scan the focus, and stop at the sharpest position.
    #[action]
    async fn focus(&self, ctx: ActionCtx) -> Result<Value, ActionError> {
        if let Some(lamp) = self.lamp.get() {
            lamp.switch_on().await?;
        }
        let mut best = (f64::MIN, 0);
        for z in -5..=5 {
            ctx.check_cancelled()?;
            self.stage.move_to(z).await?;
            let sharpness = self.camera.sharpness(z).await?;
            if sharpness > best.0 {
                best = (sharpness, z);
            }
        }
        self.stage.move_to(best.1).await?;
        Ok(json!({
            "best_z": best.1,
            "lamp": self.lamp.get().map(|l| ThingRef::name(l).to_owned()),
            "cameras": self.cameras.keys().collect::<Vec<_>>(),
        }))
    }
}

fn registry() -> ThingRegistry {
    ThingRegistry::new()
        .register::<Stage>("lab.stage:Stage")
        .register::<SimCamera>("lab.cameras:SimCamera")
        .register::<RealCamera>("lab.cameras:RealCamera")
        .register::<Lamp>("lab.lamp:Lamp")
        .register::<Autofocus>("lab.autofocus:Autofocus")
}

async fn focus_with(title: &str, config: Value) -> anyhow::Result<Value> {
    let settings = tempfile::tempdir()?;
    let mut config = config;
    config["settings_folder"] = json!(settings.path());
    println!("{title}\n  {}", serde_json::to_string(&config["things"])?);
    let config = ServerConfig::from_value(&config)?;
    let server = ThingServer::from_config(&config, &registry())?.build()?;
    println!("  start order: {:?}", server.runtime().start_order());
    let client = TestClient::start(server).await?;
    let autofocus = client
        .runtime()
        .thing_ref::<Autofocus>("autofocus")
        .expect("configured");
    let result = autofocus.focus().await.map_err(ActionError::into_anyhow)?;
    println!("  focus() → {result}\n");
    client.stop().await;
    Ok(result)
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
    let simulated = focus_with(
        "1. The simulated camera, found by its default name; no lamp:",
        json!({"things": {
            "autofocus": "lab.autofocus:Autofocus",
            "stage": "lab.stage:Stage",
            "camera": "lab.cameras:SimCamera",
        }}),
    )
    .await?;
    anyhow::ensure!(simulated["best_z"] == 3 && simulated["lamp"].is_null());

    let real = focus_with(
        "2. The configuration swaps in the real camera, and adds a lamp:",
        json!({"things": {
            "autofocus": {"class": "lab.autofocus:Autofocus", "thing_slots": {"camera": "real"}},
            "stage": "lab.stage:Stage",
            "simulated": "lab.cameras:SimCamera",
            "real": "lab.cameras:RealCamera",
            "lamp": "lab.lamp:Lamp",
        }}),
    )
    .await?;
    anyhow::ensure!(real["best_z"] == -2 && real["lamp"] == "lamp");

    println!("3. A configuration that can't work is refused:");
    let broken = ServerConfig::from_value(&json!({"things": {
        "autofocus": "lab.autofocus:Autofocus",
        "stage": "lab.stage:Stage",
        "real": "lab.cameras:RealCamera",
    }}))?;
    let error = ThingServer::from_config(&broken, &registry())?
        .build()
        .unwrap_err();
    println!("  {error}");
    Ok(())
}
