//! Units and semantic annotations: `unit` on properties, `@type`
//! on the Thing and its affordances, and the `@context` prefixes that make
//! compact terms like `saref:Sensor` resolvable.

use std::process::ExitCode;

use teta_wot::prelude::*;
use teta_wot::testing::TestClient;

/// A thermometer, described with SAREF and OM terms.
#[derive(Thing)]
#[thing(
    semantic_type = "saref:Sensor",
    context(
        saref = "https://saref.etsi.org/core/",
        om = "http://www.ontology-of-units-of-measure.org/resource/om-2/"
    )
)]
pub struct Thermometer {
    /// The measured temperature.
    #[property(
        default = 21.0,
        readonly,
        unit = "om:degreeCelsius",
        semantic_type = "saref:Temperature"
    )]
    temperature: Prop<f64>,

    /// How often to measure.
    #[property(default = 1.0, gt = 0, unit = "om:second")]
    interval: Prop<f64>,
}

#[thing_impl]
impl Thermometer {
    /// Take a measurement now.
    #[action(semantic_type = "saref:GetCommand")]
    async fn measure(&self) -> f64 {
        self.temperature.get()
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
        .thing("thermometer", Thermometer::default())
        .build()?;
    let client = TestClient::start(server).await?;
    let td = client.get("/thermometer/").await.json();

    println!(
        "@context: {}",
        serde_json::to_string_pretty(&td["@context"])?
    );
    println!("@type:    {}", td["@type"]);
    for name in ["temperature", "interval"] {
        let p = &td["properties"][name];
        println!(
            "{name}: unit {}, @type {}",
            p["unit"],
            p.get("@type").unwrap_or(&"-".into())
        );
    }
    println!("measure: @type {}", td["actions"]["measure"]["@type"]);

    // The TD, with its additions, is still valid TD 1.1.
    let parsed: teta_wot::td::ThingDescription = serde_json::from_value(td.clone())?;
    parsed.validate()?;
    println!("The TD validates against the W3C TD 1.1 JSON Schema.");
    anyhow::ensure!(td["properties"]["temperature"]["unit"] == "om:degreeCelsius");
    client.stop().await;
    Ok(())
}
