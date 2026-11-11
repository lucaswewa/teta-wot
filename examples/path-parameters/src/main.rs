//! Path parameters: custom endpoints that take values from the path
//! itself.
//!
//! A slide scanner serves the tiles of an image pyramid at
//! `tiles/{level}/{tile}`, where a tile is `{column}_{row}.txt`, and the
//! files of a scan, at any depth, at `files/{*path}`. `files/index.txt` is
//! a literal path, and a literal beats a parameter, which beats a wildcard:
//! it lists the files rather than looking for one called `index.txt`.

use std::collections::BTreeMap;
use std::process::ExitCode;

use teta_wot::http::PathParams;
use teta_wot::http::axum::http::StatusCode;
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// How many levels the pyramid has.
const LEVELS: u32 = 4;

/// A scan's files, by path.
fn scan_files() -> BTreeMap<String, String> {
    [
        ("scan.json", r#"{"tiles": 42, "objective": "20x"}"#),
        (
            "notes/overview.txt",
            "Tissue in the upper left; a bubble at 3_4.",
        ),
        ("notes/2026-11-11.txt", "Rescanned row 4."),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_owned(), text.to_owned()))
    .collect()
}

/// A slide scanner's results.
#[derive(Thing)]
pub struct Slide {
    #[thing(init = scan_files())]
    files: BTreeMap<String, String>,
}

#[thing_impl]
impl Slide {
    /// How many levels the image pyramid has.
    #[property]
    async fn levels(&self) -> u32 {
        LEVELS
    }

    /// One tile of the pyramid, such as `tiles/2/3_4.txt`.
    #[endpoint(get, "tiles/{level}/{tile}")]
    async fn tile(&self, params: PathParams) -> Response {
        let level = params.parse::<u32>("level").filter(|level| *level < LEVELS);
        let position = params
            .get("tile")
            .and_then(|tile| tile.strip_suffix(".txt"))
            .and_then(|tile| tile.split_once('_'));
        match (level, position) {
            (Some(level), Some((column, row))) => {
                format!("the tile at level {level}, column {column}, row {row}").into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }

    /// A file of the scan, at any depth: the wildcard gets the rest of the
    /// path, slashes included.
    #[endpoint(get, "files/{*path}")]
    async fn file(&self, params: PathParams) -> Response {
        match params.get("path").and_then(|path| self.files.get(path)) {
            Some(text) => text.clone().into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    /// The list of files. Its literal path beats the wildcard.
    #[endpoint(get, "files/index.txt")]
    async fn index(&self) -> String {
        self.files.keys().cloned().collect::<Vec<_>>().join(", ")
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
    let server = ThingServer::builder()
        .thing("slide", Slide::default())
        .build()?;
    let client = TestClient::start(server).await?;

    for (path, expected) in [
        ("/slide/tiles/2/3_4.txt", 200),
        ("/slide/tiles/9/0_0.txt", 404),
        ("/slide/files/notes/overview.txt", 200),
        ("/slide/files/notes%2F2026-11-11.txt", 200),
        ("/slide/files/index.txt", 200),
        ("/slide/files/missing.txt", 404),
    ] {
        let response = client.get(path).await;
        println!("GET {path} → {}: {}", response.status, response.text());
        anyhow::ensure!(response.status.as_u16() == expected, "expected {expected}");
    }

    let openapi = client.get("/openapi.json").await.json();
    for path in ["/slide/tiles/{level}/{tile}", "/slide/files/{path}"] {
        let names: Vec<_> = openapi["paths"][path]["get"]["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|parameter| parameter["name"].as_str())
            .collect();
        println!("OpenAPI: GET {path} has the path parameters {names:?}");
        anyhow::ensure!(!names.is_empty(), "no parameters listed for {path}");
    }
    Ok(())
}
