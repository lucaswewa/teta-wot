//! The optional global lock.
//!
//! With the lock enabled, actions and client property writes take it, so
//! they happen one at a time; anything that can't get it within 50 ms is
//! refused. The lock belongs to an invocation, not a thread, so an action
//! can take it again. Actions can opt out.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use teta_wot::prelude::*;

struct Microscope {
    exposure: Prop<f64>,
}

impl Thing for Microscope {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Microscope")
            .property("exposure", DataProperty::new(|t: &Microscope| &t.exposure))
            .action(
                "acquire",
                Action::new(
                    |_t: Arc<Microscope>, ctx: ActionCtx, _: NoInput| async move {
                        // The invocation already holds the lock; taking it again
                        // (as a nested call would) doesn't block.
                        let _again = ctx.hold_global_lock().await?;
                        ctx.sleep(Duration::from_millis(300)).await?;
                        Ok::<_, ActionError>("image acquired")
                    },
                ),
            )
            .action(
                "autofocus",
                Action::new(
                    |_t: Arc<Microscope>, ctx: ActionCtx, _: NoInput| async move {
                        ctx.sleep(Duration::from_millis(10)).await?;
                        Ok::<_, ActionError>(())
                    },
                ),
            )
            .action(
                "status",
                Action::new(
                    |t: Arc<Microscope>, _ctx: ActionCtx, _: NoInput| async move {
                        Ok::<_, ActionError>(format!("exposure {} ms", t.exposure.get()))
                    },
                )
                // Harmless to run at any time: opt out of the lock.
                .global_lock(false),
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
    let _ = teta_wot::logging::init(false);
    let runtime = Runtime::builder()
        .global_lock(true)
        .thing(
            "scope",
            Microscope {
                exposure: Prop::new(10.0),
            },
        )
        .build()?;
    runtime.start().await?;
    let scope = runtime.thing("scope").expect("added above");

    let acquire = scope
        .action("acquire")
        .expect("defined")
        .invoke(Value::Null)?;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // While `acquire` holds the lock, another action can't start…
    let focus = scope
        .action("autofocus")
        .expect("defined")
        .invoke(Value::Null)?;
    assert_eq!(focus.wait().await, InvocationStatus::Error);
    let problem = focus.error().expect("a problem");
    println!(
        "autofocus: {} ({}), log: {:?}",
        problem.title.unwrap_or_default(),
        problem.status.unwrap_or_default(),
        focus.logs().last().map(|r| r.message.clone()),
    );

    // …and a client can't write a property (HTTP 409 in Phase 3)…
    let error = scope
        .property("exposure")
        .expect("defined")
        .write(json!(20.0))
        .await
        .expect_err("busy");
    println!(
        "PUT exposure: {} (status {:?})",
        error,
        error.problem().status
    );

    // …but an action that opted out runs.
    let status = scope
        .action("status")
        .expect("defined")
        .invoke(Value::Null)?;
    assert_eq!(status.wait().await, InvocationStatus::Completed);
    println!("status: {}", status.output().unwrap_or(Value::Null));

    // `acquire` took the lock twice (reentrant) and finishes normally.
    assert_eq!(acquire.wait().await, InvocationStatus::Completed);
    println!("acquire: {}", acquire.output().unwrap_or(Value::Null));

    // Once it is released, everything works again.
    scope
        .property("exposure")
        .expect("defined")
        .write(json!(20.0))
        .await?;
    let focus = scope
        .action("autofocus")
        .expect("defined")
        .invoke(Value::Null)?;
    assert_eq!(focus.wait().await, InvocationStatus::Completed);
    println!("after release: exposure set, autofocus {}", focus.status());

    runtime.stop().await;
    Ok(())
}
