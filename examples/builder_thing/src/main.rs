//! A Thing defined with the builder API.
//!
//! - a read-only data property (`position`) and a constrained, writable one
//!   (`settle_ms`);
//! - a functional property (`filter`) computed by a getter;
//! - an action with typed input and output (`select`), and one without
//!   input (`home`);
//! - start and stop hooks.
//!
//! It then drives the Thing the way the HTTP binding will: through the
//! registry, with JSON values.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use teta_wot::TdOptions;
use teta_wot::prelude::*;

/// The filters in the wheel, by position.
const FILTERS: [&str; 6] = ["open", "red", "green", "blue", "ND 1", "ND 2"];

struct FilterWheel {
    position: Prop<u8>,
    settle_ms: Prop<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectInput {
    /// The position to move to, from 0 to 5.
    position: u8,
}

#[derive(Serialize, JsonSchema)]
struct Selected {
    position: u8,
    filter: String,
}

impl Thing for FilterWheel {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("FilterWheel")
            .description("A six-position filter wheel.")
            .property(
                "position",
                DataProperty::new(|t: &FilterWheel| &t.position)
                    .read_only()
                    .doc("The current position."),
            )
            .property(
                "settle_ms",
                DataProperty::new(|t: &FilterWheel| &t.settle_ms)
                    .doc("Settling time.\n\nHow long to wait after each move.")
                    .unit("ms"),
            )
            .property(
                "filter",
                FunctionalProperty::getter(|t: Arc<FilterWheel>| async move {
                    Ok::<_, PropertyError>(FILTERS[usize::from(t.position.get())].to_owned())
                })
                .doc("The filter in the beam."),
            )
            .action(
                "select",
                Action::new(
                    |t: Arc<FilterWheel>, ctx: ActionCtx, input: SelectInput| async move {
                        if usize::from(input.position) >= FILTERS.len() {
                            return Err(ActionError::handled(format!(
                                "There is no position {}.",
                                input.position
                            )));
                        }
                        ctx.sleep(Duration::from_millis(t.settle_ms.get())).await?;
                        t.position.set(input.position)?;
                        Ok(Selected {
                            position: input.position,
                            filter: FILTERS[usize::from(input.position)].to_owned(),
                        })
                    },
                )
                .doc("Move to a position.\n\nReturns the position and the filter there."),
            )
            .action(
                "home",
                Action::new(
                    |t: Arc<FilterWheel>, _ctx: ActionCtx, _: NoInput| async move {
                        t.position.set(0)?;
                        Ok::<_, ActionError>(())
                    },
                )
                .doc("Move to position 0."),
            )
    }

    async fn start(self: Arc<Self>, ctx: ThingCtx) -> anyhow::Result<()> {
        eprintln!("[{}] starting", ctx.name());
        Ok(())
    }

    async fn stop(self: Arc<Self>, ctx: ThingCtx) {
        eprintln!("[{}] stopping", ctx.name());
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
    let wheel = FilterWheel {
        position: Prop::new(0),
        settle_ms: Prop::new(10).with_constraints(Constraints::new().le(1000)),
    };
    let runtime = Runtime::builder().thing("wheel", wheel).build()?;
    runtime.start().await?;
    let wheel = runtime.thing("wheel").expect("added above");

    // Properties, as a client sees them.
    let settle = wheel.property("settle_ms").expect("defined");
    settle.write(json!("20")).await?; // lax coercion: "20" is accepted
    let too_long = settle
        .write(json!(5000))
        .await
        .expect_err("above the maximum");
    eprintln!("writing 5000 to settle_ms: {too_long}");

    // Actions: invoke, then wait for the invocation.
    let invocation = wheel
        .action("select")
        .expect("defined")
        .invoke(json!({"position": 2}))?;
    assert_eq!(invocation.wait().await, InvocationStatus::Completed);
    println!("{}", serde_json::to_string_pretty(&invocation.record())?);
    assert_eq!(
        wheel.property("filter").expect("defined").read().await?,
        json!("green")
    );

    let refused = wheel
        .action("select")
        .expect("defined")
        .invoke(json!({"position": 9}))?;
    assert_eq!(refused.wait().await, InvocationStatus::Error);
    eprintln!(
        "select 9: {}",
        refused.error().and_then(|e| e.detail).unwrap_or_default()
    );

    let home = wheel
        .action("home")
        .expect("defined")
        .invoke(serde_json::Value::Null)?;
    assert_eq!(home.wait().await, InvocationStatus::Completed);

    // The Thing Description comes from the same definition.
    let td = wheel.thing_description(&TdOptions::for_name("wheel"))?;
    td.validate_schema()?;
    println!("{}", serde_json::to_string_pretty(&td)?);

    runtime.stop().await;
    Ok(())
}
