//! `Prop<T>`: the typed cells that hold a Thing's data properties.
//!
//! Shows constraints and the pydantic-shaped errors they produce, change
//! observers (a Rust watcher and the message broker a client would use),
//! lax coercion of client values, and what `read_only` does and doesn't
//! restrict.

use std::process::ExitCode;
use std::sync::Arc;

use serde_json::json;
use teta_wot::prelude::*;

struct Heater {
    /// Set point in °C, 20 to 90.
    target: Prop<f64>,
    /// A label, lower-case letters only.
    label: Prop<String>,
    /// Firmware version: clients may read it, only the Thing sets it.
    firmware: Prop<String>,
}

impl Thing for Heater {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Heater")
            .property(
                "target",
                DataProperty::new(|t: &Heater| &t.target)
                    .doc("Set point.")
                    .unit("degree Celsius"),
            )
            .property(
                "label",
                DataProperty::new(|t: &Heater| &t.label).doc("A short label."),
            )
            .property(
                "firmware",
                DataProperty::new(|t: &Heater| &t.firmware)
                    .read_only()
                    .doc("Firmware version."),
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
    let heater = Heater {
        target: Prop::new(37.0).with_constraints(Constraints::new().ge(20).le(90)),
        label: Prop::new("incubator".to_owned()).with_constraints(
            Constraints::new()
                .min_length(1)
                .max_length(16)
                .pattern("^[a-z]+$"),
        ),
        firmware: Prop::new("1.0.0".to_owned()),
    };
    let runtime = Runtime::builder().thing("heater", heater).build()?;
    let handle = runtime.thing("heater").expect("added above");
    let heater: Arc<Heater> = handle.instance().expect("a Heater");

    // 1. Rust code sets values; constraints are checked, types by the compiler.
    let mut watcher = heater.target.subscribe();
    heater.target.set(42.5)?;
    assert!(watcher.has_changed()?);
    println!("watcher saw target = {}", *watcher.borrow_and_update());

    let error = heater.target.set(120.0).expect_err("above the maximum");
    println!("set(120.0) failed: {error}");
    assert_eq!(
        heater.target.get(),
        42.5,
        "an invalid value is never stored"
    );

    // 2. Clients write JSON through the registry, and get pydantic's errors.
    let target = handle.property("target").expect("defined");
    let mut broker = runtime.broker().subscribe("heater", "target");
    target.write(json!("55")).await?; // "55" is coerced to 55.0, as pydantic does
    println!(
        "broker saw target = {}",
        broker.recv().await.expect("published").payload
    );

    for bad in [json!(10), json!("hot"), json!(null)] {
        let Err(PropertyError::Invalid(error)) = target.write(bad.clone()).await else {
            anyhow::bail!("{bad} should have been refused");
        };
        println!(
            "PUT {bad} → 422 {}",
            serde_json::to_string(&json!({"detail": error.issues}))?
        );
    }

    let label = handle.property("label").expect("defined");
    let Err(PropertyError::Invalid(error)) = label.write(json!("Lab 3")).await else {
        anyhow::bail!("an upper-case label should be refused");
    };
    println!(
        "PUT \"Lab 3\" → 422 {}",
        serde_json::to_string(&error.issues)?
    );

    // 3. Read-only restricts clients, not the Thing.
    let firmware = handle.property("firmware").expect("defined");
    assert!(matches!(
        firmware.write(json!("2.0.0")).await,
        Err(PropertyError::ReadOnly(_))
    ));
    assert!(!firmware.is_resettable(), "clients can't reset it either");
    heater.firmware.set("1.1.0".to_owned())?;
    println!("firmware is now {}", firmware.read().await?);

    // 4. Reset goes back to the default.
    target.reset().await?;
    assert_eq!(target.read().await?, json!(37.0));
    println!("target reset to {}", target.read().await?);
    Ok(())
}
