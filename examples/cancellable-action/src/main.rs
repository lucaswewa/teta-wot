//! Cooperative cancellation.
//!
//! Three long actions on a simulated scanner, each cancelled from another
//! task the way `DELETE /action_invocations/{id}` will cancel them:
//!
//! 1. `scan` checks for cancellation while it works and stops (`cancelled`);
//! 2. `scan_or_save` handles the cancellation, saves what it has and
//!    finishes normally (`completed`);
//! 3. `batch` runs a child invocation; cancelling the parent cancels the
//!    child, and the parent ends `cancelled` once the child has stopped.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teta_wot::prelude::*;

struct Scanner {
    lines: Prop<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScanInput {
    lines: u32,
}

impl Thing for Scanner {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Scanner")
            .property(
                "lines",
                DataProperty::new(|t: &Scanner| &t.lines).read_only(),
            )
            .action(
                "scan",
                Action::new(
                    |t: Arc<Scanner>, ctx: ActionCtx, input: ScanInput| async move {
                        for line in 0..input.lines {
                            // `?` ends the invocation as `cancelled` if a cancel arrived.
                            ctx.sleep(Duration::from_millis(10)).await?;
                            t.lines.set(line + 1)?;
                        }
                        Ok::<_, ActionError>(input.lines)
                    },
                )
                .doc("Scan some lines, stopping if cancelled."),
            )
            .action(
                "scan_or_save",
                Action::new(
                    |t: Arc<Scanner>, ctx: ActionCtx, input: ScanInput| async move {
                        for line in 0..input.lines {
                            if let Err(Cancelled) = ctx.sleep(Duration::from_millis(10)).await {
                                // The cancellation was consumed: carry on, and finish.
                                tracing::info!("cancelled after {line} lines; saving them");
                                return Ok::<_, ActionError>(
                                    json!({"saved": line, "complete": false}),
                                );
                            }
                            t.lines.set(line + 1)?;
                        }
                        Ok(json!({"saved": input.lines, "complete": true}))
                    },
                )
                .doc("Scan some lines, saving the partial scan if cancelled."),
            )
            .action(
                "batch",
                Action::new(|_t: Arc<Scanner>, ctx: ActionCtx, _: NoInput| async move {
                    let child = ctx.spawn_child(|child| async move {
                        for _ in 0..1000 {
                            child.sleep(Duration::from_millis(10)).await?;
                        }
                        Ok::<_, ActionError>(())
                    });
                    tracing::info!("waiting for child invocation {}", child.id());
                    child.join().await
                })
                .doc("Run a long child invocation."),
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
        .thing(
            "scanner",
            Scanner {
                lines: Prop::new(0),
            },
        )
        .build()?;
    runtime.start().await?;
    let scanner = runtime.thing("scanner").expect("added above");

    let cases = [
        ("scan", json!({"lines": 1000}), InvocationStatus::Cancelled),
        (
            "scan_or_save",
            json!({"lines": 1000}),
            InvocationStatus::Completed,
        ),
        ("batch", Value::Null, InvocationStatus::Cancelled),
    ];
    for (action, input, expected) in cases {
        let invocation = scanner.action(action).expect("defined").invoke(input)?;
        tokio::time::sleep(Duration::from_millis(55)).await;
        // What `DELETE /action_invocations/{id}` does:
        runtime.invocations().cancel(invocation.id())?;
        let status = invocation.wait().await;
        println!(
            "{action}: {status}, output {}, error {}",
            invocation.output().unwrap_or(Value::Null),
            invocation.error().and_then(|e| e.title).unwrap_or_default(),
        );
        assert_eq!(status, expected, "{action}");
        // A finished invocation can't be cancelled again.
        println!(
            "  cancelling again: {}",
            runtime.invocations().cancel(invocation.id()).unwrap_err()
        );
    }

    runtime.stop().await;
    Ok(())
}
