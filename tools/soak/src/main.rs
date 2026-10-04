//! The soak test: drives the simulated microscope with every
//! kind of client for as long as it is given, and watches its memory.
//!
//! ```text
//! cargo build --release -p simulated-microscope -p soak
//! target/release/soak --server target/release/simulated-microscope.exe --duration 24h --report soak.csv
//! ```
//!
//! The server runs as its own process, so that its memory is measured
//! alone. The clients, all at once, on real sockets:
//!
//! - property reads and writes, including the camera's rendered sharpness;
//! - stage moves, waited for, and long moves cancelled;
//! - autofocus runs;
//! - image captures, whose Blobs are downloaded;
//! - two MJPEG viewers, two server-sent events streams and a WebSocket,
//!   which must never end.
//!
//! Every minute it prints (and with `--report` appends to a CSV file) the
//! requests, errors, latency, frames, events, WebSocket messages, the
//! server's working set, private bytes and handle count, and the invocations it keeps. It
//! exits with a failure if any request failed, a stream ended, or the
//! private bytes grew after the warm-up: the median of the last quarter of
//! the run against the first quarter's, beyond 25 % plus 16 MB.
//!
//! Options: `--duration` (such as `90s`, `10m`, `24h`; 5 minutes by
//! default), `--profile tetathing|wot`, `--warmup` (by default a fifth of the
//! run, at most 10 minutes), `--report FILE`.

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

// ---- Options -----------------------------------------------------------------

struct Options {
    server: PathBuf,
    duration: Duration,
    warmup: Option<Duration>,
    profile: String,
    report: Option<PathBuf>,
}

fn parse_duration(text: &str) -> anyhow::Result<Duration> {
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let number: u64 = number
        .parse()
        .with_context(|| format!("a duration, not `{text}`"))?;
    let seconds = match unit {
        "" | "s" => number,
        "m" => number * 60,
        "h" => number * 3600,
        other => bail!("unknown unit `{other}` in `{text}`: use s, m or h"),
    };
    Ok(Duration::from_secs(seconds))
}

fn options() -> anyhow::Result<Options> {
    let mut options = Options {
        server: PathBuf::from("target/release/simulated-microscope.exe"),
        duration: Duration::from_secs(300),
        warmup: None,
        profile: "tetathing".into(),
        report: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--server" => options.server = value()?.into(),
            "--duration" => options.duration = parse_duration(&value()?)?,
            "--warmup" => options.warmup = Some(parse_duration(&value()?)?),
            "--profile" => options.profile = value()?,
            "--report" => options.report = Some(value()?.into()),
            other => bail!("unknown option `{other}`"),
        }
    }
    Ok(options)
}

// ---- A keep-alive HTTP/1.1 client ------------------------------------------

/// One connection, reused for every request, as a browser or `requests`
/// session would: thousands of short connections a minute would fill
/// Windows' ephemeral ports with sockets in `TIME_WAIT`.
struct Http {
    at: SocketAddr,
    connection: Option<BufReader<TcpStream>>,
}

impl Http {
    fn new(at: SocketAddr) -> Self {
        Self {
            at,
            connection: None,
        }
    }

    async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Vec<u8>)> {
        // A connection the server closed while idle is reopened once.
        match self.try_request(method, path, body).await {
            Ok(response) => Ok(response),
            Err(_) => {
                self.connection = None;
                self.try_request(method, path, body).await
            }
        }
    }

    async fn try_request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Vec<u8>)> {
        if self.connection.is_none() {
            self.connection = Some(BufReader::new(TcpStream::connect(self.at).await?));
        }
        let connection = self.connection.as_mut().expect("connected");
        let body = body.map(Value::to_string).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.at,
            body.len()
        );
        connection.get_mut().write_all(head.as_bytes()).await?;
        connection.get_mut().write_all(body.as_bytes()).await?;

        let mut line = String::new();
        connection.read_line(&mut line).await?;
        let status: u16 = line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .with_context(|| format!("not an HTTP status line: {line:?}"))?;
        let (mut length, mut chunked, mut close) = (0usize, false, false);
        loop {
            line.clear();
            connection.read_line(&mut line).await?;
            let header = line.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                let value = value.trim();
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => length = value.parse().unwrap_or(0),
                    "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
                    "connection" => close = value.eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        let mut body = Vec::new();
        if chunked {
            loop {
                line.clear();
                connection.read_line(&mut line).await?;
                let size = usize::from_str_radix(line.trim(), 16).context("a chunk size")?;
                let mut chunk = vec![0; size + 2];
                connection.read_exact(&mut chunk).await?;
                if size == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..size]);
            }
        } else {
            body.resize(length, 0);
            connection.read_exact(&mut body).await?;
        }
        if close {
            self.connection = None;
        }
        Ok((status, body))
    }

    async fn json(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Value)> {
        let (status, body) = self.request(method, path, body).await?;
        Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    }
}

// ---- Statistics ----------------------------------------------------------------

#[derive(Default)]
struct Stats {
    requests: AtomicU64,
    errors: AtomicU64,
    frames: AtomicU64,
    events: AtomicU64,
    messages: AtomicU64,
    stream_ends: AtomicU64,
    latencies: Mutex<Vec<Duration>>,
    first_errors: Mutex<Vec<String>>,
}

impl Stats {
    fn error(&self, what: String) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        let mut first = self.first_errors.lock().unwrap_or_else(|e| e.into_inner());
        if first.len() < 20 {
            eprintln!("error: {what}");
            first.push(what);
        }
    }

    /// Times a request, counting it and any failure.
    async fn time<T>(
        &self,
        what: &str,
        request: impl Future<Output = anyhow::Result<(u16, T)>>,
    ) -> Option<T> {
        let started = Instant::now();
        let result = request.await;
        self.latencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(started.elapsed());
        self.requests.fetch_add(1, Ordering::Relaxed);
        match result {
            Ok((status, value)) if (200..300).contains(&status) => Some(value),
            Ok((status, _)) => {
                self.error(format!("{what}: status {status}"));
                None
            }
            Err(error) => {
                self.error(format!("{what}: {error:#}"));
                None
            }
        }
    }
}

// ---- The clients -------------------------------------------------------------

/// Invokes an action and waits for it to finish: its final status.
async fn invoke(http: &mut Http, stats: &Stats, path: &str, input: Value) -> Option<Value> {
    let body = (!input.is_null()).then_some(&input);
    let started = stats.time(path, http.json("POST", path, body)).await?;
    let status_path = local(started["href"].as_str()?);
    loop {
        let status = stats
            .time(&status_path, http.json("GET", &status_path, None))
            .await?;
        match status["status"].as_str() {
            Some("pending" | "running") => tokio::time::sleep(Duration::from_millis(25)).await,
            _ => return Some(status),
        }
    }
}

/// The path of an absolute URL on the server.
fn local(href: &str) -> String {
    href.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|i| rest[i..].to_owned()))
        .unwrap_or_else(|| href.to_owned())
}

async fn properties(at: SocketAddr, stats: Arc<Stats>, running: Arc<AtomicBool>) {
    let mut http = Http::new(at);
    let mut n = 0u64;
    while running.load(Ordering::Relaxed) {
        stats
            .time(
                "GET /stage/position",
                http.json("GET", "/stage/position", None),
            )
            .await;
        stats
            .time(
                "GET /autofocus/last_focus",
                http.json("GET", "/autofocus/last_focus", None),
            )
            .await;
        stats
            .time(
                "PUT /camera/fps",
                http.request("PUT", "/camera/fps", Some(&json!(10.0 + (n % 3) as f64))),
            )
            .await;
        if n.is_multiple_of(10) {
            stats
                .time(
                    "GET /camera/sharpness",
                    http.json("GET", "/camera/sharpness", None),
                )
                .await;
        }
        n += 1;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn moves(at: SocketAddr, stats: Arc<Stats>, running: Arc<AtomicBool>, profile: String) {
    let mut http = Http::new(at);
    let mut n = 0i64;
    while running.load(Ordering::Relaxed) {
        n += 1;
        if n % 10 == 0 {
            // A long move, cancelled.
            let Some(started) = stats
                .time(
                    "POST /stage/move_to",
                    http.json(
                        "POST",
                        "/stage/move_to",
                        Some(&json!({"x": 100_000, "y": 0, "z": 0})),
                    ),
                )
                .await
            else {
                continue;
            };
            let path = local(started["href"].as_str().unwrap_or_default());
            tokio::time::sleep(Duration::from_millis(100)).await;
            stats
                .time("DELETE invocation", http.request("DELETE", &path, None))
                .await;
            let expected = if profile == "wot" {
                "failed"
            } else {
                "cancelled"
            };
            loop {
                let Some(status) = stats
                    .time("GET invocation", http.json("GET", &path, None))
                    .await
                else {
                    break;
                };
                match status["status"].as_str() {
                    Some("pending" | "running") => {
                        tokio::time::sleep(Duration::from_millis(25)).await
                    }
                    Some(s) if s == expected => break,
                    other => {
                        stats.error(format!("a cancelled move ended {other:?}"));
                        break;
                    }
                }
            }
            invoke(&mut http, &stats, "/stage/home", Value::Null).await;
        } else {
            let dz = if n % 2 == 0 { 20 } else { -20 };
            if let Some(status) =
                invoke(&mut http, &stats, "/stage/move_by", json!({"dz": dz})).await
                && status["status"] != "completed"
            {
                stats.error(format!("a move ended {}", status["status"]));
            }
        }
    }
}

async fn autofocus(at: SocketAddr, stats: Arc<Stats>, running: Arc<AtomicBool>) {
    let mut http = Http::new(at);
    while running.load(Ordering::Relaxed) {
        if let Some(status) = invoke(
            &mut http,
            &stats,
            "/autofocus/run",
            json!({"range": 200, "steps": 5}),
        )
        .await
            && status["status"] != "completed"
        {
            stats.error(format!("an autofocus ended {}", status["status"]));
        }
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
}

async fn captures(at: SocketAddr, stats: Arc<Stats>, running: Arc<AtomicBool>) {
    let mut http = Http::new(at);
    while running.load(Ordering::Relaxed) {
        if let Some(status) = invoke(&mut http, &stats, "/camera/capture", Value::Null).await {
            match status["output"]["href"].as_str() {
                Some(href) => {
                    let path = local(href);
                    if let Some(jpeg) = stats
                        .time("GET a Blob", http.request("GET", &path, None))
                        .await
                        && !jpeg.starts_with(&[0xff, 0xd8])
                    {
                        stats.error("a Blob that isn't a JPEG".into());
                    }
                }
                None => stats.error(format!("a capture ended {}", status["status"])),
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// A long response (MJPEG or SSE), counting `marker`s into `counter`, and
/// reopened if the server ends it.
async fn stream(
    at: SocketAddr,
    path: &'static str,
    accept: &'static str,
    marker: &'static [u8],
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    events: bool,
) {
    while running.load(Ordering::Relaxed) {
        let Ok(mut socket) = TcpStream::connect(at).await else {
            stats.error(format!("GET {path}: can't connect"));
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let head = format!("GET {path} HTTP/1.1\r\nHost: {at}\r\nAccept: {accept}\r\n\r\n");
        if socket.write_all(head.as_bytes()).await.is_err() {
            continue;
        }
        let mut buffer = vec![0; 64 * 1024];
        let mut tail: Vec<u8> = Vec::new();
        loop {
            let read = tokio::select! {
                read = socket.read(&mut buffer) => read,
                () = tokio::time::sleep(Duration::from_secs(1)) => {
                    if running.load(Ordering::Relaxed) { continue } else { return }
                }
            };
            match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    tail.extend_from_slice(&buffer[..n]);
                    let found = tail.windows(marker.len()).filter(|w| *w == marker).count() as u64;
                    let counter = if events { &stats.events } else { &stats.frames };
                    counter.fetch_add(found, Ordering::Relaxed);
                    let keep = tail.len().saturating_sub(marker.len() - 1);
                    tail.drain(..keep);
                }
            }
        }
        if running.load(Ordering::Relaxed) {
            stats.stream_ends.fetch_add(1, Ordering::Relaxed);
            stats.error(format!("GET {path} ended"));
        }
    }
}

async fn websocket(at: SocketAddr, stats: Arc<Stats>, running: Arc<AtomicBool>) {
    while running.load(Ordering::Relaxed) {
        let Ok((mut socket, _)) =
            tokio_tungstenite::connect_async(format!("ws://{at}/stage/ws")).await
        else {
            stats.error("the WebSocket can't open".into());
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let subscribe = r#"{"messageType": "addPropertyObservation", "data": {"position": true}}"#;
        if socket.send(Message::text(subscribe)).await.is_err() {
            continue;
        }
        loop {
            let message = tokio::select! {
                message = socket.next() => message,
                () = tokio::time::sleep(Duration::from_secs(1)) => {
                    if running.load(Ordering::Relaxed) { continue } else { return }
                }
            };
            match message {
                Some(Ok(Message::Text(_))) => {
                    stats.messages.fetch_add(1, Ordering::Relaxed);
                }
                Some(Ok(_)) => {}
                _ => break,
            }
        }
        if running.load(Ordering::Relaxed) {
            stats.stream_ends.fetch_add(1, Ordering::Relaxed);
            stats.error("the WebSocket closed".into());
        }
    }
}

// ---- The server --------------------------------------------------------------

/// The server's memory, from PowerShell.
struct Memory {
    /// The working set, in bytes: what is in RAM now. Windows trims it, so
    /// it jumps.
    working_set: u64,
    /// The private bytes: what the process has allocated. It only grows if
    /// something is kept, so it is what the leak check looks at.
    private: u64,
    /// The open handles (sockets, files, threads, events).
    handles: u64,
}

fn memory(pid: u32) -> Option<Memory> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "$p = Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PrivateMemorySize64) $($p.HandleCount)\""
            ),
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut numbers = text.split_whitespace().filter_map(|n| n.parse().ok());
    Some(Memory {
        working_set: numbers.next()?,
        private: numbers.next()?,
        handles: numbers.next()?,
    })
}

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index].as_secs_f64() * 1000.0
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<bool> {
    let options = options()?;
    let folder = tempfile::tempdir()?;
    let config = json!({
        "things": {
            "stage": "microscope.stage:Stage",
            "camera": "microscope.camera:Camera",
            "autofocus": "microscope.autofocus:Autofocus",
        },
        "settings_folder": folder.path().join("settings"),
        "wire_profile": options.profile,
    });
    let config_file = folder.path().join("microscope.json");
    std::fs::write(&config_file, config.to_string())?;
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port();
    let log = std::fs::File::create(folder.path().join("server.log"))?;
    let mut server = std::process::Command::new(&options.server)
        .arg("-c")
        .arg(&config_file)
        .args(["--port", &port.to_string()])
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .with_context(|| format!("starting {}", options.server.display()))?;
    let at = SocketAddr::from(([127, 0, 0, 1], port));
    let mut http = Http::new(at);
    let deadline = Instant::now() + Duration::from_secs(60);
    while http.request("GET", "/stage/", None).await.is_err() {
        if Instant::now() > deadline {
            let _ = server.kill();
            bail!("the server didn't answer within a minute");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let warmup = options
        .warmup
        .unwrap_or((options.duration / 5).min(Duration::from_secs(600)));
    println!(
        "soaking {} (the {} profile) for {:?}, warm-up {:?}, pid {}",
        options.server.display(),
        options.profile,
        options.duration,
        warmup,
        server.id()
    );

    let stats = Arc::new(Stats::default());
    let running = Arc::new(AtomicBool::new(true));
    let mut clients = Vec::new();
    let (s, r) = (&stats, &running);
    clients.push(tokio::spawn(properties(at, Arc::clone(s), Arc::clone(r))));
    clients.push(tokio::spawn(properties(at, Arc::clone(s), Arc::clone(r))));
    clients.push(tokio::spawn(moves(
        at,
        Arc::clone(s),
        Arc::clone(r),
        options.profile.clone(),
    )));
    clients.push(tokio::spawn(autofocus(at, Arc::clone(s), Arc::clone(r))));
    clients.push(tokio::spawn(captures(at, Arc::clone(s), Arc::clone(r))));
    for _ in 0..2 {
        clients.push(tokio::spawn(stream(
            at,
            "/camera/preview",
            "*/*",
            b"--frame",
            Arc::clone(s),
            Arc::clone(r),
            false,
        )));
    }
    clients.push(tokio::spawn(stream(
        at,
        "/stage/arrived",
        "text/event-stream",
        b"data:",
        Arc::clone(s),
        Arc::clone(r),
        true,
    )));
    clients.push(tokio::spawn(stream(
        at,
        "/camera/captured",
        "text/event-stream",
        b"data:",
        Arc::clone(s),
        Arc::clone(r),
        true,
    )));
    clients.push(tokio::spawn(websocket(at, Arc::clone(s), Arc::clone(r))));

    let mut report = match &options.report {
        Some(path) => {
            let mut file = std::fs::File::create(path)?;
            writeln!(
                file,
                "minute,requests,errors,p50_ms,p99_ms,frames,events,ws_messages,working_set_mb,private_mb,handles,invocations"
            )?;
            Some(file)
        }
        None => None,
    };
    println!(
        "minute  requests errors   p50 ms   p99 ms   frames  events  ws msgs  working MB  private MB  handles  invocations"
    );
    let started = Instant::now();
    let mut samples: Vec<(Duration, f64)> = Vec::new();
    let mut minute = 0;
    while started.elapsed() < options.duration {
        let remaining = options.duration.saturating_sub(started.elapsed());
        tokio::time::sleep(remaining.min(Duration::from_secs(60))).await;
        minute += 1;
        let mut latencies =
            std::mem::take(&mut *stats.latencies.lock().unwrap_or_else(|e| e.into_inner()));
        latencies.sort();
        let take = |counter: &AtomicU64| counter.swap(0, Ordering::Relaxed);
        let (requests, errors) = (take(&stats.requests), stats.errors.load(Ordering::Relaxed));
        let (frames, events, messages) = (
            take(&stats.frames),
            take(&stats.events),
            take(&stats.messages),
        );
        let Memory {
            working_set,
            private,
            handles,
        } = memory(server.id()).unwrap_or(Memory {
            working_set: 0,
            private: 0,
            handles: 0,
        });
        let (working, private_mb) = (
            working_set as f64 / 1_048_576.0,
            private as f64 / 1_048_576.0,
        );
        let invocations = match http.json("GET", "/action_invocations", None).await {
            Ok((_, Value::Array(list))) => list.len(),
            _ => 0,
        };
        if started.elapsed() >= warmup && private > 0 {
            samples.push((started.elapsed(), private_mb));
        }
        let (p50, p99) = (percentile(&latencies, 0.5), percentile(&latencies, 0.99));
        println!(
            "{minute:>6} {requests:>9} {errors:>6} {p50:>8.1} {p99:>8.1} {frames:>8} {events:>7} {messages:>8} {working:>11.1} {private_mb:>11.1} {handles:>8} {invocations:>12}"
        );
        if let Some(file) = &mut report {
            writeln!(
                file,
                "{minute},{requests},{errors},{p50:.2},{p99:.2},{frames},{events},{messages},{working:.2},{private_mb:.2},{handles},{invocations}"
            )?;
        }
        if server.try_wait()?.is_some() {
            stats.error("the server exited".into());
            break;
        }
    }

    running.store(false, Ordering::SeqCst);
    for client in clients {
        let _ = tokio::time::timeout(Duration::from_secs(30), client).await;
    }
    let _ = server.kill();
    let _ = server.wait();

    // The verdict.
    let errors = stats.errors.load(Ordering::Relaxed);
    let stream_ends = stats.stream_ends.load(Ordering::Relaxed);
    let mut passed = errors == 0 && stream_ends == 0;
    println!("\n{errors} errors, {stream_ends} streams ended early");
    if samples.len() >= 4 {
        let quarter = samples.len() / 4;
        let mut first: Vec<f64> = samples[..quarter.max(1)].iter().map(|s| s.1).collect();
        let mut last: Vec<f64> = samples[samples.len() - quarter.max(1)..]
            .iter()
            .map(|s| s.1)
            .collect();
        let (first, last) = (median(&mut first), median(&mut last));
        let limit = first * 1.25 + 16.0;
        let grew = last > limit;
        println!(
            "private bytes after the warm-up: {first:.1} MB at first, {last:.1} MB at the end (limit {limit:.1} MB): {}",
            if grew { "GREW" } else { "steady" }
        );
        passed &= !grew;
    } else {
        println!(
            "too short after the warm-up to judge memory ({} samples)",
            samples.len()
        );
    }
    println!("{}", if passed { "PASSED" } else { "FAILED" });
    Ok(passed)
}
