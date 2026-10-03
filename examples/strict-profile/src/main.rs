//! One Thing, served in both wire profiles.
//!
//! The same requests go to two servers built from the same Thing: one in
//! the default `tetathing` profile, and
//! one in the `wot` profile, which follows the W3C WoT HTTP Basic and SSE
//! Profiles where the two conflict. The example prints each answer side by
//! side, and checks the differences.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::ensure;
use serde_json::{Value, json};
use teta_wot::prelude::*;
use teta_wot::server::WireProfile;
use teta_wot::testing::{TestClient, TestResponse};

/// A dimmable lamp.
#[derive(Thing)]
pub struct Lamp {
    /// The brightness, in percent.
    #[property(default = 50, ge = 0, le = 100)]
    level: Prop<i64>,
}

#[thing_impl]
impl Lamp {
    /// Fade to a level, slowly.
    #[action]
    async fn fade(&self, level: i64) -> Result<i64, ActionError> {
        cancellable_sleep(Duration::from_millis(100)).await?;
        self.level.set(level)?;
        Ok(level)
    }

    /// Wait until cancelled.
    #[action]
    async fn wait(&self) -> Result<(), ActionError> {
        cancellable_sleep(Duration::from_secs(60)).await?;
        Ok(())
    }

    /// Double a number. It's quick, so it is synchronous: in the `wot`
    /// profile, the caller gets the answer at once.
    #[action(synchronous)]
    async fn double(&self, n: i64) -> i64 {
        2 * n
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

async fn serve(profile: WireProfile) -> anyhow::Result<TestClient> {
    let server = ThingServer::builder()
        .wire_profile(profile)
        .thing("lamp", Lamp::default())
        .build()?;
    Ok(TestClient::start(server).await?)
}

/// A response in one line: status, content type and body, shortened.
fn line(response: &TestResponse) -> String {
    let mut body = response.text();
    if body.len() > 70 {
        let cut = (0..=70)
            .rev()
            .find(|i| body.is_char_boundary(*i))
            .unwrap_or(0);
        body = format!("{}…", &body[..cut]);
    }
    let media_type = response.header("content-type").unwrap_or("-");
    format!("{} {media_type} {body}", response.status.as_u16())
}

fn show(request: &str, tetathing: &TestResponse, wot: &TestResponse) {
    println!("{request}");
    println!("  tetathing: {}", line(tetathing));
    println!("  wot:       {}", line(wot));
}

async fn run() -> anyhow::Result<()> {
    let tetathing = serve(WireProfile::TetaThing).await?;
    let wot = serve(WireProfile::Wot).await?;

    // Writing a property: 201 and `null`, or 204.
    let (l, w) = (
        tetathing.put_json("/lamp/level", &json!(30)).await,
        wot.put_json("/lamp/level", &json!(30)).await,
    );
    show("PUT /lamp/level 30", &l, &w);
    ensure!(l.status == 201 && w.status == 204 && w.body.is_empty());

    // An invalid value: FastAPI's 422, or a 400 problem.
    let (l, w) = (
        tetathing.put_json("/lamp/level", &json!(500)).await,
        wot.put_json("/lamp/level", &json!(500)).await,
    );
    show("PUT /lamp/level 500", &l, &w);
    ensure!(l.status == 422 && w.status == 400);
    ensure!(w.header("content-type") == Some("application/problem+json"));
    println!("  wot's invalid-params: {}", w.json()["invalid-params"]);

    // Invoking: TetaThing's invocation, or the Profile's ActionStatus.
    let input = json!({"level": 80});
    let (l, w) = (
        tetathing.post_json("/lamp/fade", Some(&input)).await,
        wot.post_json("/lamp/fade", Some(&input)).await,
    );
    show("POST /lamp/fade {\"level\": 80}", &l, &w);
    ensure!(l.status == 201 && w.status == 201);
    let keys = |v: Value| -> Vec<String> {
        v.as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .collect()
    };
    println!("  tetathing' fields: {}", keys(l.json()).join(", "));
    println!("  wot's fields:      {}", keys(w.json()).join(", "));

    // …and once it has finished.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let path = |r: &TestResponse| {
        r.header("location")
            .unwrap_or_default()
            .replace("http://testserver", "")
    };
    let (l, w) = (tetathing.get(&path(&l)).await, wot.get(&path(&w)).await);
    show("GET the invocation, finished", &l, &w);
    ensure!(l.json()["status"] == "completed" && l.json()["timeCompleted"].is_string());
    ensure!(
        w.json()["status"] == "completed"
            && w.json()["timeEnded"]
                .as_str()
                .is_some_and(|t| t.ends_with('Z'))
    );
    println!(
        "  tetathing' time: {} (naive local time)",
        l.json()["timeCompleted"]
    );
    println!(
        "  wot's time:      {} (RFC 3339, UTC)",
        w.json()["timeEnded"]
    );

    // A synchronous action: 201 and an invocation, or 200 and the output.
    let (l, w) = (
        tetathing
            .post_json("/lamp/double", Some(&json!({"n": 21})))
            .await,
        wot.post_json("/lamp/double", Some(&json!({"n": 21}))).await,
    );
    show("POST /lamp/double {\"n\": 21}", &l, &w);
    ensure!(l.status == 201 && w.status == 200 && w.json() == 42);

    // Cancelling: 200 and `null`, or 204; the invocation ends `cancelled`,
    // or `failed` with a problem.
    let (l, w) = (
        tetathing.post_json("/lamp/wait", None).await,
        wot.post_json("/lamp/wait", None).await,
    );
    let (l_path, w_path) = (path(&l), path(&w));
    let (l, w) = (tetathing.delete(&l_path).await, wot.delete(&w_path).await);
    show("DELETE the invocation", &l, &w);
    ensure!(l.status == 200 && w.status == 204);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (l, w) = (tetathing.get(&l_path).await, wot.get(&w_path).await);
    println!("  then tetathing: {}", l.json()["status"]);
    println!(
        "  then wot:       {} ({})",
        w.json()["status"],
        w.json()["error"]["title"]
    );
    ensure!(l.json()["status"] == "cancelled" && w.json()["status"] == "failed");

    // An unknown path.
    let (l, w) = (tetathing.get("/nothing").await, wot.get("/nothing").await);
    show("GET /nothing", &l, &w);
    ensure!(w.header("content-type") == Some("application/problem+json"));

    // The TDs: the `wot` profile's names the Profiles, and says which
    // actions are synchronous.
    let (l, w) = (
        tetathing.get("/lamp/").await.json(),
        wot.get("/lamp/").await.json(),
    );
    println!("TD");
    println!(
        "  tetathing: profile {}, double synchronous: {}",
        l.get("profile").unwrap_or(&Value::Null),
        l["actions"]["double"]["synchronous"]
    );
    println!(
        "  wot:       profile {}, double synchronous: {}",
        w["profile"], w["actions"]["double"]["synchronous"]
    );
    ensure!(w["actions"]["double"]["synchronous"] == true);
    ensure!(l["actions"]["double"]["synchronous"] == false);

    // What doesn't differ: the new top-level resources answer the W3C way
    // in both profiles.
    let (l, w) = (
        tetathing.get("/lamp/properties").await,
        wot.get("/lamp/properties").await,
    );
    show("GET /lamp/properties (readallproperties)", &l, &w);
    ensure!(l.body == w.body);

    tetathing.stop().await;
    wot.stop().await;
    println!("\nThe profiles differ where expected.");
    Ok(())
}
