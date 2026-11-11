//! The simulated microscope as a Windows service.
//!
//! A Windows service exists only on Windows, so on other systems this
//! program just says so. On Linux, run `simulated-microscope -c
//! microscope.json` under systemd instead: `shutdown_signal` turns the
//! `SIGTERM` that systemd sends into the same graceful stop.

use std::process::ExitCode;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() -> ExitCode {
    windows::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!(
        "microscope-service is a Windows service, so it runs only on Windows. \
         On Linux, run `simulated-microscope -c microscope.json` under systemd: \
         it stops gracefully on SIGTERM."
    );
    ExitCode::SUCCESS
}
