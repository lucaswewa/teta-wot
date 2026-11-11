//! Stopping a server from outside, as a service manager or a terminal does.
//!
//! On Linux, `shutdown_signal` turns `SIGTERM` (`kill`, systemd), `SIGINT`
//! (Ctrl-C) and `SIGHUP` (the terminal closing) into a graceful stop: the
//! server logs which signal arrived, stops its Things, logs `stopped`, and
//! exits with success. Each test runs the simulated microscope as its own
//! process, on a port of its own, in a temporary folder.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long starting, or stopping, may take before a test gives up.
const DEADLINE: Duration = Duration::from_secs(60);

/// The server's process, killed if a test fails before it has exited.
struct Server {
    child: Child,
    lines: Receiver<String>,
    printed: Vec<String>,
    _folder: tempfile::TempDir,
}

impl Server {
    fn start() -> Self {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/microscope.json");
        let mut child = Command::new(env!("CARGO_BIN_EXE_simulated-microscope"))
            .args(["-c", config, "--port", "0"])
            // The configuration's settings folder is relative to this one.
            .current_dir(folder.path())
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .spawn()
            .expect("the server starts");
        let stdout = child.stdout.take().expect("its standard output");
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            printed: Vec::new(),
            _folder: folder,
        }
    }

    /// Waits for a line the server prints that `wanted` accepts, and
    /// returns it.
    fn wait_for(&mut self, what: &str, wanted: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.printed.push(line.clone());
                    if wanted(&line) {
                        return line;
                    }
                }
                Err(error) => panic!(
                    "the server never printed a line {what} ({error}); it printed:\n{}",
                    self.printed.join("\n")
                ),
            }
        }
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .args(["-s", name, &self.child.id().to_string()])
            .status()
            .expect("`kill` runs");
        assert!(status.success(), "`kill -s {name}` failed");
    }

    fn exit_status(&mut self) -> ExitStatus {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().expect("the server's status") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the server didn't exit within {DEADLINE:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sends one HTTP request, so the test knows the server is serving.
fn answers_http(address: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect(address) else {
        return false;
    };
    let request = format!("GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    let mut answer = String::new();
    stream.write_all(request.as_bytes()).is_ok()
        && stream.read_to_string(&mut answer).is_ok()
        && answer.starts_with("HTTP/1.1 ")
}

fn stops_gracefully_on(signal: &str, logged: &str) {
    let mut server = Server::start();
    let listening = server.wait_for("saying where it listens", |line| {
        line.starts_with("listening on http://")
    });
    let address = listening.trim_start_matches("listening on http://");
    assert!(
        answers_http(address),
        "the server doesn't answer at {address}"
    );

    server.signal(signal);
    server.wait_for(&format!("ending `{logged}`"), |line| line.ends_with(logged));
    server.wait_for("saying it stopped", |line| {
        line.ends_with("teta_wot_server: stopped")
    });
    let status = server.exit_status();
    assert!(
        status.success(),
        "the server exited with {status}; it printed:\n{}",
        server.printed.join("\n")
    );
}

#[test]
fn sigterm_stops_the_server_gracefully() {
    stops_gracefully_on("TERM", "received SIGTERM");
}

#[test]
fn sigint_stops_the_server_gracefully() {
    stops_gracefully_on("INT", "received SIGINT (Ctrl-C)");
}

#[test]
fn sighup_stops_the_server_gracefully() {
    stops_gracefully_on("HUP", "received SIGHUP: the terminal closed");
}
