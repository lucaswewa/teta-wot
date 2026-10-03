//! Builds the Thing Description of an LED illuminator by hand, prints it, and
//! validates it: first the TD rules the builder checks, then the W3C TD 1.1
//! JSON Schema.

use std::process::ExitCode;

use serde_json::json;
use teta_wot::td::{
    ActionAffordance, Constraints, DataSchema, EventAffordance, Form, Link, Operation,
    PropertyAffordance, TdError, ThingDescription,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `/{thing}/{name}` with the given operations.
fn form(name: &str, ops: impl IntoIterator<Item = Operation>) -> Form {
    Form::new(format!("/illuminator/{name}")).with_op(ops.into_iter().collect::<Vec<_>>())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // A data schema can be written by hand...
    let mut brightness = DataSchema::for_type::<u8>()?;
    Constraints::new().le(100).apply(&mut brightness)?;

    let td = ThingDescription::builder("Illuminator")
        .id("urn:uuid:6f1e4bd2-2f3c-5b1a-9d7e-3c1f0a2b4c5d")
        .description("A dimmable LED illuminator for a microscope.")
        .base("http://localhost:5000/")
        // Semantic annotations need a prefix in the @context.
        .context_prefix("saref", "https://saref.etsi.org/core/")
        .semantic_type("saref:Actuator")
        .property(
            "brightness",
            PropertyAffordance::builder(brightness)
                .title("Brightness")
                .description("LED brightness, as a percentage of full power.")
                .unit("percent")
                .default_value(json!(50))
                .form(form(
                    "brightness",
                    [Operation::ReadProperty, Operation::WriteProperty],
                )),
        )
        .property(
            "temperature",
            PropertyAffordance::builder(DataSchema::for_type::<f64>()?)
                .title("LED temperature")
                .read_only(true)
                .unit("degree Celsius")
                .semantic_type("saref:Temperature")
                .form(form("temperature", [Operation::ReadProperty])),
        )
        .action(
            "toggle",
            ActionAffordance::builder()
                .title("Switch the LED on or off.")
                .output(
                    DataSchema::for_type::<bool>()?.with_description("Whether the LED is now on."),
                )
                .form(form("toggle", [Operation::InvokeAction])),
        )
        .event(
            "overheated",
            EventAffordance::builder()
                .title("The LED got too hot and was switched off.")
                .data(DataSchema::for_type::<f64>()?.with_unit("degree Celsius"))
                .form(
                    form(
                        "overheated",
                        [Operation::SubscribeEvent, Operation::UnsubscribeEvent],
                    )
                    .with_subprotocol("sse")
                    .with_content_type("text/event-stream"),
                ),
        )
        .link(
            Link::new("/camera/mjpeg_stream")
                .with_rel("alternate")
                .with_media_type("multipart/x-mixed-replace"),
        )
        .build()?;

    println!("{}", serde_json::to_string_pretty(&td)?);

    // The builder already checked the TD rules; now the W3C JSON Schema.
    td.validate_schema()?;
    eprintln!("\nThe TD validates against the W3C TD 1.1 JSON Schema.");

    // The builder refuses TDs that break the rules, for example a form that
    // uses an action operation on a property.
    let error = ThingDescription::builder("Broken")
        .property(
            "brightness",
            PropertyAffordance::builder(DataSchema::for_type::<u8>()?)
                .form(form("brightness", [Operation::InvokeAction])),
        )
        .build()
        .expect_err("a property form can't invoke an action");
    assert!(matches!(error, TdError::MisplacedOperation { .. }));
    eprintln!("A TD with a misplaced operation is refused: {error}");
    Ok(())
}
