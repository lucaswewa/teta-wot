//! Exporting the API description without starting a server: the OpenAPI
//! document, which code generators read, and every Thing Description.
//!
//! ```text
//! cargo run -p openapi-export                                   # to target/openapi-export
//! cargo run -p openapi-export -- out --base http://lab-pc:5000/ # another folder and host
//! ```
//!
//! Then generate a TypeScript client from the document:
//!
//! ```text
//! npx --yes openapi-typescript@7 target/openapi-export/openapi.json -o target/openapi-export/api.d.ts
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use teta_wot::blob::Jpeg;
use teta_wot::prelude::*;
use teta_wot::server::HttpOptions;

/// A position, in steps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    /// Along x.
    x: i64,
    /// Along y.
    y: i64,
    /// Along z (focus).
    z: i64,
}

/// A motorised stage.
#[derive(Thing)]
pub struct Stage {
    /// Where the stage is.
    #[property(readonly)]
    position: Prop<Position>,

    /// The stage reached a new position.
    #[event]
    moved: Event<Position>,
}

#[thing_impl]
impl Stage {
    /// Move to a position.
    #[action]
    async fn move_to(
        &self,
        x: i64,
        y: i64,
        #[param(default = 0)] z: i64,
    ) -> Result<Position, ActionError> {
        let position = Position { x, y, z };
        self.position.set(position)?;
        self.moved.emit(position);
        Ok(position)
    }
}

/// A camera.
#[derive(Thing)]
pub struct Camera {
    /// The exposure time.
    #[property(default = 10.0, gt = 0, unit = "ms")]
    exposure: Prop<f64>,

    #[stream]
    preview: MjpegStream,
}

#[thing_impl]
impl Camera {
    /// Capture an image.
    #[action]
    async fn capture(&self) -> Result<Blob<Jpeg>, ActionError> {
        Ok(Blob::from_bytes(vec![0xff, 0xd8, 0xff, 0xd9]))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut folder = PathBuf::from("target/openapi-export");
    let mut base = "http://localhost:5000/".to_owned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => {
                base = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--base needs a URL"))?
            }
            other => folder = PathBuf::from(other),
        }
    }

    // The runtime is built, not started: nothing runs, nothing listens.
    let runtime = Runtime::builder()
        .thing("stage", Stage::default())
        .thing("camera", Camera::default())
        .build()?;
    let options = HttpOptions {
        api_title: "Microscope".into(),
        api_version: "1.0.0".into(),
        ..HttpOptions::default()
    };

    std::fs::create_dir_all(&folder)?;
    let document = teta_wot::server::openapi(&runtime, &options);
    let path = folder.join("openapi.json");
    std::fs::write(&path, serde_json::to_string_pretty(&document)? + "\n")?;
    let operations: usize = document["paths"]
        .as_object()
        .map(|paths| {
            paths
                .values()
                .filter_map(|item| item.as_object())
                .map(|item| item.len())
                .sum()
        })
        .unwrap_or_default();
    println!("wrote {} ({operations} operations)", path.display());

    for thing in runtime.things() {
        let td = teta_wot::server::thing_description(&runtime, thing.name(), &base, &options)
            .map_err(anyhow::Error::msg)?;
        let path = folder.join(format!("{}.td.json", thing.name()));
        std::fs::write(&path, serde_json::to_string_pretty(&td)? + "\n")?;
        println!("wrote {} (base {base})", path.display());
    }

    for (path, method) in [("/stage/move_to", "post"), ("/camera/exposure", "put")] {
        println!(
            "  {} {path}: {}",
            method.to_uppercase(),
            document["paths"][path][method]["operationId"]
        );
    }
    anyhow::ensure!(
        document["paths"]["/stage/move_to"]["post"]["operationId"]
            == "start_action_stage_move_to_post"
    );
    Ok(())
}
