//! Invocation logs.
//!
//! Every invocation runs in a `tracing` span; the invocation log layer copies
//! the events inside it into the invocation's log, which clients read when
//! they poll the invocation. Events from a device closure count too, because
//! the device thread re-enters the caller's span.

use std::process::ExitCode;
use std::sync::Arc;

use serde_json::Value;
use teta_wot::prelude::*;

/// A pump driver: synchronous, on its own thread.
struct Pump;

impl Pump {
    fn prime(&mut self) -> anyhow::Result<u32> {
        tracing::info!("priming the pump");
        tracing::debug!("valve states: open, closed, open"); // below INFO: not captured
        Ok(3)
    }
}

impl Driver for Pump {}

struct Dispenser {
    pump: Device<Pump>,
}

impl Thing for Dispenser {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Dispenser")
            .device("pump", |t: &Dispenser| &t.pump)
            .action(
                "dispense",
                Action::new(
                    |t: Arc<Dispenser>, _ctx: ActionCtx, _: NoInput| async move {
                        tracing::info!("dispensing");
                        let strokes = t.pump.try_call(|pump, _| pump.prime()).await?;
                        tracing::warn!(strokes, "priming took more than one stroke");
                        Ok::<_, ActionError>(strokes)
                    },
                ),
            )
            .action(
                "jam",
                Action::new(
                    |_t: Arc<Dispenser>, _ctx: ActionCtx, _: NoInput| async move {
                        tracing::info!("dispensing");
                        let cause = anyhow::anyhow!("pressure 3.2 bar above the limit");
                        Err::<(), _>(ActionError::from(cause.context("the nozzle is jammed")))
                    },
                ),
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
    // Console output plus the invocation log layer. Without the layer,
    // invocation logs stay empty.
    teta_wot::logging::init(false)?;

    let dispenser = Dispenser {
        pump: Device::new(|| Ok(Pump)),
    };
    let runtime = Runtime::builder().thing("dispenser", dispenser).build()?;
    runtime.start().await?;
    let thing = runtime.thing("dispenser").expect("added above");

    let ok = thing
        .action("dispense")
        .expect("defined")
        .invoke(Value::Null)?;
    ok.wait().await;
    let log = ok.logs();
    println!("dispense log:\n{}", serde_json::to_string_pretty(&log)?);
    let messages: Vec<_> = log.iter().map(|r| r.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "dispensing",
            "priming the pump",
            "priming took more than one stroke strokes=3"
        ],
        "the device thread's event is in the log; DEBUG is not"
    );
    assert_eq!(log[2].levelname, "WARNING");

    let failed = thing.action("jam").expect("defined").invoke(Value::Null)?;
    failed.wait().await;
    let last = failed.logs().pop().expect("the error is logged");
    println!(
        "jam: [{}] {}\ntraceback:\n{}",
        last.exception_type.as_deref().unwrap_or("-"),
        last.message,
        last.traceback.as_deref().unwrap_or("-"),
    );
    assert_eq!(last.levelname, "ERROR");
    assert!(
        last.traceback.is_some(),
        "errors are logged with their cause chain"
    );

    runtime.stop().await;
    Ok(())
}
