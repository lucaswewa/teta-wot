//! A synchronous driver behind a device actor.
//!
//! The driver simulates a serial-port instrument. It holds an `Rc`, so it
//! isn't `Send`, like many vendor SDK handles: it works because it is
//! created, used and dropped on its own thread. The example shows concurrent
//! async callers, a blocking caller on a plain thread, a call timeout, a
//! panic that faults the device, recovery with `reset`, and an out-of-band
//! abort.

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use teta_wot::prelude::*;
use teta_wot::{AbortHandle, DeviceState};

/// A pretend serial port: every command takes 2 ms and is logged.
struct SerialInstrument {
    transcript: Rc<RefCell<Vec<String>>>,
    busy_until_abort: Arc<AtomicBool>,
}

impl SerialInstrument {
    fn query(&mut self, command: &str) -> String {
        std::thread::sleep(Duration::from_millis(2));
        self.transcript.borrow_mut().push(command.to_owned());
        format!("{command}: ok (#{})", self.transcript.borrow().len())
    }
}

impl Driver for SerialInstrument {
    fn open(&mut self) -> anyhow::Result<()> {
        println!(
            "open on thread {:?}",
            std::thread::current().name().unwrap_or("?")
        );
        Ok(())
    }

    fn close(&mut self) -> anyhow::Result<()> {
        println!("close after {} commands", self.transcript.borrow().len());
        Ok(())
    }

    fn abort_handle(&self) -> Option<AbortHandle> {
        let flag = Arc::clone(&self.busy_until_abort);
        Some(Arc::new(move || flag.store(false, Ordering::SeqCst)))
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
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
    let busy = Arc::new(AtomicBool::new(false));
    let driver_busy = Arc::clone(&busy);
    let options = DeviceOptions {
        call_timeout: Some(Duration::from_millis(200)),
        ..DeviceOptions::default()
    };
    // The factory runs on the device thread, so the driver never has to move.
    let device = Arc::new(Device::with_options(
        move || {
            Ok(SerialInstrument {
                transcript: Rc::default(),
                busy_until_abort: Arc::clone(&driver_busy),
            })
        },
        options,
    ));
    // A Thing's devices are opened by the runtime; here we do it by hand.
    device.open("example:serial").await?;

    // 1. Many async callers: the mailbox serialises them.
    let callers: Vec<_> = (0..20)
        .map(|i| {
            let device = Arc::clone(&device);
            tokio::spawn(async move {
                device
                    .call(move |d, _| d.query(&format!("READ? {i}")))
                    .await
            })
        })
        .collect();
    for caller in callers {
        caller.await??;
    }
    let count = device.call(|d, _| d.transcript.borrow().len()).await?;
    println!("20 concurrent callers → {count} commands, one at a time");
    assert_eq!(count, 20);

    // 2. A blocking caller on an ordinary thread (a capture loop, say).
    let worker = Arc::clone(&device);
    let reply = std::thread::spawn(move || worker.call_blocking(|d, _| d.query("*IDN?")))
        .join()
        .expect("no panic")?;
    println!("from a plain thread: {reply}");

    // 3. A call that doesn't answer in time; an abort stops it out of band.
    busy.store(true, Ordering::SeqCst);
    let stuck = device.call(|d, _| {
        while d.busy_until_abort.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        "unstuck"
    });
    let error = stuck.await.expect_err("times out");
    println!("stuck call: {error}");
    assert!(device.abort(), "the driver provides an abort handle");
    assert_eq!(
        device.call(|_, _| "responsive again").await?,
        "responsive again"
    );

    // 4. A panic faults the device; later calls fail fast until a reset.
    let error = device
        .call(|_, _| -> () { panic!("firmware bug") })
        .await
        .expect_err("panics");
    println!("panicking call: {error}");
    assert_eq!(device.state(), DeviceState::Faulted);
    let error = device
        .call(|d, _| d.query("*IDN?"))
        .await
        .expect_err("faulted");
    println!("next call: {error}");
    device.reset().await?;
    assert_eq!(device.state(), DeviceState::Ready);
    println!(
        "after reset: {}",
        device.call(|d, _| d.query("*IDN?")).await?
    );

    device.close().await?;
    Ok(())
}
