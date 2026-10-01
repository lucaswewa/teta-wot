//! Integration tests for device actors and their public lifecycle API.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId;
use std::time::Duration;

use teta_wot_core::{
    AbortHandle, Device, DeviceError, DeviceOptions, DeviceState, Driver, InvocationScope,
};
use tokio::sync::oneshot;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap()
}

// Rc deliberately makes this driver !Send: all lifecycle methods must stay
// on the thread that constructs it.
struct TestDriver {
    value: Rc<Cell<usize>>,
    events: Arc<Mutex<Vec<(&'static str, ThreadId)>>>,
    aborts: Arc<AtomicUsize>,
}

impl Driver for TestDriver {
    fn open(&mut self) -> anyhow::Result<()> {
        lock(&self.events).push(("open", std::thread::current().id()));
        Ok(())
    }

    fn close(&mut self) -> anyhow::Result<()> {
        lock(&self.events).push(("close", std::thread::current().id()));
        Ok(())
    }

    fn abort_handle(&self) -> Option<AbortHandle> {
        let aborts = self.aborts.clone();
        Some(Arc::new(move || {
            aborts.fetch_add(1, Ordering::SeqCst);
        }))
    }
}

#[derive(Default)]
struct PlainDriver;
impl Driver for PlainDriver {}

#[tokio::test]
async fn lifecycle_and_reset_keep_non_send_driver_on_one_named_thread() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let aborts = Arc::new(AtomicUsize::new(0));
    let device = Device::with_options(
        {
            let events = events.clone();
            let aborts = aborts.clone();
            move || {
                lock(&events).push(("create", std::thread::current().id()));
                Ok(TestDriver {
                    value: Rc::new(Cell::new(0)),
                    events: events.clone(),
                    aborts: aborts.clone(),
                })
            }
        },
        DeviceOptions {
            thread_init: Some(Arc::new({
                let events = events.clone();
                move || lock(&events).push(("init", std::thread::current().id()))
            })),
            ..DeviceOptions::default()
        },
    );
    assert_eq!(device.name(), "device");
    assert_eq!(device.state(), DeviceState::Closed);
    assert_eq!(device.queue_depth(), 0);
    assert!(lock(&events).is_empty());
    assert!(!device.abort());
    device.close().await.unwrap();
    device.open("thing:motor").await.unwrap();
    assert_eq!(device.name(), "thing:motor");
    assert_eq!(device.state(), DeviceState::Ready);
    let (thread_id, thread_name) = device
        .call(|driver, token| {
            assert!(!token.is_cancelled());
            driver.value.set(42);
            lock(&driver.events).push(("call", std::thread::current().id()));
            (
                std::thread::current().id(),
                std::thread::current().name().unwrap().to_owned(),
            )
        })
        .await
        .unwrap();
    assert_ne!(thread_id, std::thread::current().id());
    assert_eq!(thread_name, "device:thing:motor");
    assert_eq!(device.call(|d, _| d.value.get()).await.unwrap(), 42);
    assert!(device.abort());
    assert_eq!(aborts.load(Ordering::SeqCst), 1);
    device.reset().await.unwrap();
    assert_eq!(device.call(|d, _| d.value.get()).await.unwrap(), 0);
    device.close().await.unwrap();
    device.close().await.unwrap();
    assert_eq!(device.state(), DeviceState::Closed);
    assert!(!device.abort());
    let events = lock(&events);
    assert_eq!(
        events.iter().map(|e| e.0).collect::<Vec<_>>(),
        [
            "init", "create", "open", "call", "close", "create", "open", "close"
        ]
    );
    assert!(events.iter().all(|e| e.1 == thread_id));
}

#[tokio::test]
async fn closed_calls_and_duplicate_open_are_rejected() {
    let device = Device::new(|| Ok(PlainDriver));
    assert!(
        matches!(device.call(|_, _| ()).await, Err(DeviceError::NotOpen(name)) if name == "device")
    );
    assert!(matches!(device.reset().await, Err(DeviceError::NotOpen(_))));
    assert!(matches!(
        device.call_blocking(|_, _| ()),
        Err(DeviceError::NotOpen(_))
    ));
    device.open("sensor").await.unwrap();
    assert!(
        matches!(device.open("other").await, Err(DeviceError::Open { name, .. }) if name == "other")
    );
    assert_eq!(device.name(), "sensor");
    assert_eq!(device.state(), DeviceState::Ready);
    device.close().await.unwrap();
    assert!(
        matches!(device.call(|_, _| ()).await, Err(DeviceError::NotOpen(name)) if name == "sensor")
    );
}

#[tokio::test]
async fn driver_errors_do_not_fault_the_device() {
    let device = Device::new(|| Ok(PlainDriver));
    device.open("sensor").await.unwrap();
    assert_eq!(
        device
            .try_call(|_, _| Ok::<_, anyhow::Error>(7))
            .await
            .unwrap(),
        7
    );
    assert!(
        matches!(device.try_call(|_, _| Err::<(), _>(anyhow::anyhow!("read failed"))).await,
        Err(DeviceError::Driver(error)) if error.to_string() == "read failed")
    );
    assert_eq!(device.state(), DeviceState::Ready);
    assert_eq!(device.call(|_, _| 9).await.unwrap(), 9);
    device.close().await.unwrap();
}

#[tokio::test]
async fn panic_faults_calls_until_reset_recreates_the_driver() {
    let mut generation = 0;
    let device = Device::new(move || {
        generation += 1;
        Ok(CounterDriver(generation))
    });
    device.open("motor").await.unwrap();
    assert_eq!(device.call(|d, _| d.0).await.unwrap(), 1);
    assert!(matches!(device.call(|_, _| panic!("jammed")).await,
        Err(DeviceError::Panicked { name, message }) if name == "motor" && message == "jammed"));
    // A second call also synchronizes with the worker's state update after
    // the panic reply, whether it was already faulted or still in the mailbox.
    assert!(matches!(device.call(|_, _| panic!("must not run")).await,
        Err(DeviceError::Faulted { name, reason }) if name == "motor" && reason == "jammed"));
    assert_eq!(device.state(), DeviceState::Faulted);
    device.reset().await.unwrap();
    assert_eq!(device.state(), DeviceState::Ready);
    assert_eq!(device.call(|d, _| d.0).await.unwrap(), 2);
    device.close().await.unwrap();
}

struct CounterDriver(usize);
impl Driver for CounterDriver {}

#[derive(Clone, Copy)]
enum Failure {
    Error,
    Panic,
}

struct FailingDriver {
    open: Option<Failure>,
    close: Option<Failure>,
}

fn fail(failure: Option<Failure>) -> anyhow::Result<()> {
    match failure {
        None => Ok(()),
        Some(Failure::Error) => anyhow::bail!("driver failure"),
        Some(Failure::Panic) => panic!("driver panic"),
    }
}

impl Driver for FailingDriver {
    fn open(&mut self) -> anyhow::Result<()> {
        fail(self.open)
    }
    fn close(&mut self) -> anyhow::Result<()> {
        fail(self.close)
    }
}

#[tokio::test]
async fn factory_and_open_errors_and_panics_leave_device_closed() {
    for factory_failure in [false, true] {
        for failure in [Failure::Error, Failure::Panic] {
            let device = Device::new(move || {
                if factory_failure {
                    fail(Some(failure))?;
                }
                Ok(FailingDriver {
                    open: Some(failure),
                    close: None,
                })
            });
            let error = device.open("broken").await.unwrap_err();
            let expected = match failure {
                Failure::Error => "driver failure",
                Failure::Panic => "panicked: driver panic",
            };
            assert!(matches!(error, DeviceError::Open { name, source }
                if name == "broken" && source.to_string() == expected));
            assert_eq!(device.state(), DeviceState::Closed);
            assert!(matches!(
                device.call(|_, _| ()).await,
                Err(DeviceError::NotOpen(_))
            ));
            device.close().await.unwrap();
        }
    }
}

#[tokio::test]
async fn close_errors_and_panics_still_close_the_device() {
    for failure in [Failure::Error, Failure::Panic] {
        let device = Device::new(move || {
            Ok(FailingDriver {
                open: None,
                close: Some(failure),
            })
        });
        device.open("broken").await.unwrap();
        let expected = match failure {
            Failure::Error => "driver failure",
            Failure::Panic => "panicked: driver panic",
        };
        assert!(
            matches!(device.close().await, Err(DeviceError::Driver(error)) if error.to_string() == expected)
        );
        assert_eq!(device.state(), DeviceState::Closed);
        device.close().await.unwrap();
    }
}

#[tokio::test]
async fn failed_reset_can_be_retried_on_the_same_worker() {
    let mut attempts = 0;
    let device = Device::new(move || {
        attempts += 1;
        Ok(FailingDriver {
            open: (attempts == 2).then_some(Failure::Error),
            close: Some(Failure::Error),
        })
    });
    device.open("motor").await.unwrap();
    let thread = device
        .call(|_, _| std::thread::current().id())
        .await
        .unwrap();
    assert!(matches!(
        device.reset().await,
        Err(DeviceError::Open { .. })
    ));
    assert_eq!(device.state(), DeviceState::Closed);
    device.reset().await.unwrap();
    assert_eq!(
        device
            .call(|_, _| std::thread::current().id())
            .await
            .unwrap(),
        thread
    );
    assert!(matches!(device.close().await, Err(DeviceError::Driver(_))));
}

#[tokio::test]
async fn blocking_calls_return_values_errors_and_reject_reentrancy() {
    let device = Arc::new(Device::new(|| Ok(PlainDriver)));
    device.open("sensor").await.unwrap();
    let other = device.clone();
    tokio::task::spawn_blocking(move || {
        assert_eq!(other.call_blocking(|_, _| 5).unwrap(), 5);
        assert_eq!(other.try_call_blocking(|_, _| Ok::<_, anyhow::Error>(6)).unwrap(), 6);
        assert!(matches!(other.try_call_blocking(|_, _| Err::<(), _>(anyhow::anyhow!("blocking failure"))),
            Err(DeviceError::Driver(error)) if error.to_string() == "blocking failure"));
    }).await.unwrap();
    let other = device.clone();
    let result = device
        .call(move |_, _| other.call_blocking(|_, _| ()))
        .await
        .unwrap();
    assert!(matches!(result, Err(DeviceError::Reentrant(name)) if name == "sensor"));
    assert_eq!(device.state(), DeviceState::Ready);
    device.close().await.unwrap();
}

#[tokio::test]
async fn calls_receive_invocation_cancellation_and_log_in_its_span() {
    teta_wot_core::testing::init_tracing();
    let device = Device::new(|| Ok(PlainDriver));
    device.open("sensor").await.unwrap();
    let scope = InvocationScope::fake();
    scope.cancel_token().cancel();
    scope
        .clone()
        .run(device.call(|_, token| {
            assert!(token.check().is_err());
            tracing::info!("device invocation log");
        }))
        .await
        .unwrap();
    assert!(!scope.cancel_token().is_cancelled());
    assert!(
        scope
            .logs()
            .iter()
            .any(|log| log.message == "device invocation log")
    );
    assert!(!device.call(|_, token| token.is_cancelled()).await.unwrap());
    device.close().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn timeout_skips_queued_call_but_running_call_finishes() {
    let timeout = Duration::from_secs(10);
    let device = Arc::new(Device::with_options(
        || Ok(CounterDriver(0)),
        DeviceOptions {
            mailbox: 0, // Zero capacity is normalized to a usable one-slot mailbox.
            call_timeout: Some(timeout),
            ..DeviceOptions::default()
        },
    ));
    device.open("motor").await.unwrap();
    let (started, mut running) = oneshot::channel();
    let (release, wait) = sync_mpsc::channel();
    let first = device.call(move |d, _| {
        started.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(30)).unwrap();
        d.0 += 1;
    });
    tokio::pin!(first);
    // Keep the runtime runnable until the OS thread acknowledges the call;
    // otherwise Tokio can auto-advance time before that thread is scheduled.
    std::future::poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        match running.try_recv() {
            Ok(()) => std::task::Poll::Ready(()),
            Err(oneshot::error::TryRecvError::Empty) => {
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
            Err(error) => panic!("worker did not start the call: {error}"),
        }
    })
    .await;
    let second = device.call(|d, _| d.0 += 100);
    tokio::pin!(second);
    // Poll the second call into the mailbox without advancing virtual time.
    assert!(matches!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(second.as_mut().poll(cx))).await,
        std::task::Poll::Pending
    ));
    assert_eq!(device.queue_depth(), 1);
    tokio::time::advance(timeout).await;
    for result in [first.await, second.await] {
        assert!(
            matches!(result, Err(DeviceError::Timeout { name, timeout: duration })
            if name == "motor" && duration == timeout)
        );
    }
    release.send(()).unwrap();
    // Blocking calls have no timeout and queue behind both earlier calls.
    let other = device.clone();
    let value = tokio::task::spawn_blocking(move || other.call_blocking(|d, _| d.0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, 1);
    assert_eq!(device.queue_depth(), 0);
    assert_eq!(device.state(), DeviceState::Ready);
    device.close().await.unwrap();
}
