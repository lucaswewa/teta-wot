//! Derives `JsonSchema` on some Rust types and prints, for each, the JSON
//! Schema that schemars generates next to the TD DataSchema it converts to.
//!
//! Look for:
//!
//! - `$ref`s to `$defs` inlined, with a field's own doc comment winning over
//!   the doc comment of its type;
//! - `Option<T>` (`"type": [..., "null"]` or `anyOf`) turned into `oneOf`,
//!   with type-specific keywords such as `minimum` moved into their branch;
//! - tuples (`prefixItems`) turned into an `items` array;
//! - schemars' numeric formats (`uint8`, `double`) dropped, but its bounds kept;
//! - constraint arguments (`gt`, `le`, …) written as DataSchema keywords.

use std::process::ExitCode;

use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use teta_wot::td::{Constraints, ConversionError, DataSchema};

/// A rectangular region of the sensor, in pixels.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct Region {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// How the camera reads out the sensor.
#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum Readout {
    /// All rows at once.
    Global,
    /// Row by row.
    Rolling,
}

/// Settings for one acquisition.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct Acquisition {
    /// Exposure time in milliseconds.
    exposure_ms: f64,
    /// Region of interest. The whole sensor is used when this is missing.
    roi: Option<Region>,
    #[serde(default = "rolling")]
    readout: Readout,
    /// Analogue gain for each colour channel.
    gains: Vec<u8>,
    /// Horizontal and vertical binning.
    binning: (u8, u8),
    /// Optional label saved with the image.
    #[serde(default)]
    label: Option<String>,
}

fn rolling() -> Readout {
    Readout::Rolling
}

/// A tree of named nodes. It refers to itself, so it has no DataSchema.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct Node {
    name: String,
    children: Vec<Node>,
}

fn show<T: JsonSchema>(label: &str) -> Result<(), ConversionError> {
    let json_schema = serde_json::to_string_pretty(&schema_for!(T)).expect("serialises");
    let data_schema = DataSchema::for_type::<T>()?;
    println!("=== {label}\n--- JSON Schema (schemars)\n{json_schema}");
    println!(
        "--- TD DataSchema\n{}\n",
        serde_json::to_string_pretty(&data_schema).expect("serialises")
    );
    Ok(())
}

fn main() -> ExitCode {
    let result = (|| {
        show::<Option<u8>>("Option<u8>")?;
        show::<(u8, u8)>("(u8, u8)")?;
        show::<Acquisition>("Acquisition")?;

        // A property's constraints go into its schema.
        let mut exposure = DataSchema::for_type::<f64>()?;
        Constraints::new()
            .gt(0)
            .le(10_000)
            .apply(&mut exposure)
            .expect("the constraints suit a number");
        println!(
            "=== f64 with gt=0, le=10000\n{}\n",
            serde_json::to_string_pretty(&exposure).expect("serialises")
        );
        Ok::<_, ConversionError>(())
    })();
    if let Err(error) = result {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }

    // A recursive type can't be inlined; the conversion stops at the
    // recursion limit (99) with an error.
    match DataSchema::for_type::<Node>() {
        Err(error @ ConversionError::RecursionLimit { .. }) => {
            println!("=== Node (recursive)\n{error}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("error: expected a recursion error, got {other:?}");
            ExitCode::FAILURE
        }
    }
}
