//! Device actors: synchronous drivers on their own threads.
//!
//! A [`Device<D>`] owns a driver `D` on a dedicated, named OS thread, which
//! creates it, opens it, runs every call on it one at a time, and closes it.
//! The driver never leaves that thread, so it doesn't need to be `Send`,
//! which suits vendor SDKs and COM objects with thread affinity.
//!
//! Calls are closures sent through a bounded mailbox. Each closure gets the
//! driver and the calling invocation's [`CancelToken`], and runs inside the
//! caller's `tracing` span, so its log events land in the invocation's log.
//! A panicking call is caught: the device becomes [`Faulted`](DeviceState::Faulted)
//! and later calls fail at once until [`Device::reset`] succeeds.

use std::any::Any;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{JoinHandle, ThreadId};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::cancel::CancelToken;
use crate::context::{InvocationScope, panic_message};

/// A synchronous hardware driver, run by a [`Device`].
pub trait Driver: 'static {
    /// Connects to the hardware. Runs on the device thread after the driver
    /// is created, and again after a reset.
    fn open(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Disconnects cleanly (parks axes, closes ports). Runs on the device
    /// thread when the device is closed or reset.
    fn close(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// An emergency stop that works from any thread while a call is running
    /// (a blocking call can't be interrupted otherwise). Asked for after
    /// each successful open.
    fn abort_handle(&self) -> Option<AbortHandle> {
        None
    }
}

/// Stops the hardware out of band. See [`Driver::abort_handle`].
pub type AbortHandle = Arc<dyn Fn() + Send + Sync>;

/// The state of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceState {
    /// Not open (before start, after close, or after a failed open).
    Closed,
    /// Being created and opened.
    Opening,
    /// Open and taking calls.
    Ready,
    /// A call panicked; calls fail until the device is reset.
    Faulted,
    /// Being closed.
    Closing,
}

/// A device call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceError {
    /// The device isn't open.
    #[error("device `{0}` is not open")]
    NotOpen(String),
    /// The device is faulted after a panic.
    #[error("device `{name}` is faulted after a panic ({reason}); reset it")]
    Faulted {
        /// The device.
        name: String,
        /// The panic message.
        reason: String,
    },
    /// This call panicked; the device is now faulted.
    #[error("device `{name}` panicked: {message}")]
    Panicked {
        /// The device.
        name: String,
        /// The panic message.
        message: String,
    },
    /// No answer within the timeout. The call may still run later.
    #[error("device `{name}` did not answer within {timeout:?}")]
    Timeout {
        /// The device.
        name: String,
        /// The timeout.
        timeout: Duration,
    },
    /// Creating or opening the driver failed.
    #[error("device `{name}` failed to open: {source:#}")]
    Open {
        /// The device.
        name: String,
        /// Why.
        source: anyhow::Error,
    },
    /// The driver returned an error.
    #[error("{0:#}")]
    Driver(anyhow::Error),
    /// A blocking call was made from the device's own thread, which would
    /// wait for itself forever.
    #[error("device `{0}` was called from its own thread")]
    Reentrant(String),
}

/// Options for a [`Device`].
#[derive(Clone)]
pub struct DeviceOptions {
    /// How many calls may wait in the mailbox before callers wait too.
    pub mailbox: usize,
    /// How long a call waits for its answer, or `None` to wait forever.
    pub call_timeout: Option<Duration>,
    /// Runs first on the device thread, for example to initialise COM.
    pub thread_init: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Default for DeviceOptions {
    fn default() -> Self {
        Self {
            mailbox: 16,
            call_timeout: None,
            thread_init: None,
        }
    }
}

impl fmt::Debug for DeviceOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceOptions")
            .field("mailbox", &self.mailbox)
            .field("call_timeout", &self.call_timeout)
            .field("thread_init", &self.thread_init.is_some())
            .finish()
    }
}

type Factory<D> = Box<dyn FnMut() -> anyhow::Result<D> + Send>;

/// A job for the device thread.
trait Call<D>: Send {
    /// Runs on the driver; returns the panic message if it panicked.
    fn run(self: Box<Self>, driver: &mut D, name: &str) -> Option<String>;
    /// Answers without running.
    fn fail(self: Box<Self>, error: DeviceError);
    /// Whether the caller has stopped waiting.
    fn abandoned(&self) -> bool;
}

struct TypedCall<F, R> {
    f: F,
    reply: oneshot::Sender<Result<R, DeviceError>>,
    token: CancelToken,
    span: tracing::Span,
}

impl<D, F, R> Call<D> for TypedCall<F, R>
where
    F: FnOnce(&mut D, &CancelToken) -> R + Send,
    R: Send,
{
    fn run(self: Box<Self>, driver: &mut D, name: &str) -> Option<String> {
        let TypedCall {
            f,
            reply,
            token,
            span,
        } = *self;
        let result = catch_unwind(AssertUnwindSafe(|| span.in_scope(|| f(driver, &token))));
        match result {
            Ok(value) => {
                let _ = reply.send(Ok(value));
                None
            }
            Err(payload) => {
                let message = panic_message(&*payload);
                let _ = reply.send(Err(DeviceError::Panicked {
                    name: name.to_owned(),
                    message: message.clone(),
                }));
                Some(message)
            }
        }
    }

    fn fail(self: Box<Self>, error: DeviceError) {
        let _ = self.reply.send(Err(error));
    }

    fn abandoned(&self) -> bool {
        self.reply.is_closed()
    }
}

enum Job<D> {
    Call(Box<dyn Call<D>>),
    Reset(oneshot::Sender<Result<(), DeviceError>>),
    Close(oneshot::Sender<Result<(), DeviceError>>),
}

#[derive(Default)]
struct Shared {
    state: Mutex<(Option<DeviceState>, Option<String>)>,
    queued: AtomicUsize,
    thread: OnceLock<ThreadId>,
    abort: Mutex<Option<AbortHandle>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, (Option<DeviceState>, Option<String>)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn state(&self) -> DeviceState {
        self.lock().0.unwrap_or(DeviceState::Closed)
    }

    fn set(&self, state: DeviceState, reason: Option<String>) {
        *self.lock() = (Some(state), reason);
    }
}

/// A synchronous driver running on its own thread, called from async code
/// (or other threads) through a mailbox.
pub struct Device<D: Driver> {
    factory: Mutex<Option<Factory<D>>>,
    options: DeviceOptions,
    shared: Arc<Shared>,
    name: OnceLock<String>,
    sender: Mutex<Option<mpsc::Sender<Job<D>>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl<D: Driver> fmt::Debug for Device<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("name", &self.name.get())
            .field("state", &self.state())
            .field("queue_depth", &self.queue_depth())
            .finish_non_exhaustive()
    }
}

impl<D: Driver> Device<D> {
    /// A device whose driver `factory` creates. Nothing runs until the
    /// device is opened (when its Thing starts); the factory then runs on
    /// the device thread, and again on each reset.
    pub fn new(factory: impl FnMut() -> anyhow::Result<D> + Send + 'static) -> Self {
        Self::with_options(factory, DeviceOptions::default())
    }

    /// A device with options.
    pub fn with_options(
        factory: impl FnMut() -> anyhow::Result<D> + Send + 'static,
        options: DeviceOptions,
    ) -> Self {
        Self {
            factory: Mutex::new(Some(Box::new(factory))),
            options,
            shared: Arc::new(Shared::default()),
            name: OnceLock::new(),
            sender: Mutex::new(None),
            thread: Mutex::new(None),
        }
    }

    /// The device's name (`<thing>:<field>` once its Thing has started).
    pub fn name(&self) -> &str {
        self.name.get().map_or("device", String::as_str)
    }

    /// The current state.
    pub fn state(&self) -> DeviceState {
        self.shared.state()
    }

    /// How many calls are waiting in the mailbox.
    pub fn queue_depth(&self) -> usize {
        self.shared.queued.load(Ordering::Relaxed)
    }

    /// Starts the device thread (named `device:<name>`), creates the driver
    /// and opens it.
    pub async fn open(&self, name: &str) -> Result<(), DeviceError> {
        let _ = self.name.set(name.to_owned());
        let Some(factory) = lock(&self.factory).take() else {
            return Err(DeviceError::Open {
                name: name.to_owned(),
                source: anyhow::anyhow!("the device has already been opened"),
            });
        };
        let (sender, receiver) = mpsc::channel(self.options.mailbox.max(1));
        let (opened, open_result) = oneshot::channel();
        let shared = Arc::clone(&self.shared);
        let init = self.options.thread_init.clone();
        let thread_name = name.to_owned();
        shared.set(DeviceState::Opening, None);
        let handle = std::thread::Builder::new()
            .name(format!("device:{name}"))
            .spawn(move || device_thread(thread_name, factory, init, shared, receiver, opened))
            .map_err(|e| DeviceError::Open {
                name: name.to_owned(),
                source: e.into(),
            })?;
        *lock(&self.thread) = Some(handle);
        *lock(&self.sender) = Some(sender);
        open_result
            .await
            .unwrap_or_else(|_| Err(DeviceError::NotOpen(name.to_owned())))
    }

    /// Closes the driver and ends the device thread. Calls still in the
    /// mailbox run first.
    pub async fn close(&self) -> Result<(), DeviceError> {
        let Some(sender) = lock(&self.sender).take() else {
            return Ok(());
        };
        let (reply, answer) = oneshot::channel();
        let result = if sender.send(Job::Close(reply)).await.is_ok() {
            answer.await.unwrap_or(Ok(()))
        } else {
            Ok(())
        };
        drop(sender);
        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
        result
    }

    /// Recreates and reopens the driver on the same thread, for example
    /// after a panic faulted the device. The old driver is closed first if
    /// it is still there.
    pub async fn reset(&self) -> Result<(), DeviceError> {
        let sender = self.sender()?;
        let (reply, answer) = oneshot::channel();
        self.shared.queued.fetch_add(1, Ordering::Relaxed);
        if sender.send(Job::Reset(reply)).await.is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(DeviceError::NotOpen(self.name().to_owned()));
        }
        answer
            .await
            .unwrap_or_else(|_| Err(DeviceError::NotOpen(self.name().to_owned())))
    }

    /// Calls the driver's abort handle, if it has one. Returns whether it did.
    pub fn abort(&self) -> bool {
        let abort = lock(&self.shared.abort).clone();
        abort.map(|abort| abort()).is_some()
    }

    /// Runs `f` on the driver and returns its result.
    ///
    /// `f` gets the driver and the calling invocation's cancel token (a
    /// token that is never cancelled outside an invocation). If the device
    /// has a call timeout and it passes, the caller gets
    /// [`DeviceError::Timeout`]; the call is skipped if it hasn't started.
    pub async fn call<F, R>(&self, f: F) -> Result<R, DeviceError>
    where
        F: FnOnce(&mut D, &CancelToken) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (call, answer) = self.prepare(f)?;
        let sender = self.sender()?;
        self.shared.queued.fetch_add(1, Ordering::Relaxed);
        if sender.send(Job::Call(call)).await.is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(DeviceError::NotOpen(self.name().to_owned()));
        }
        let answer =
            match self.options.call_timeout {
                Some(timeout) => tokio::time::timeout(timeout, answer).await.map_err(|_| {
                    DeviceError::Timeout {
                        name: self.name().to_owned(),
                        timeout,
                    }
                })?,
                None => answer.await,
            };
        answer.unwrap_or_else(|_| Err(DeviceError::NotOpen(self.name().to_owned())))
    }

    /// Like [`call`](Self::call), for driver methods that return a
    /// `Result`: their error becomes [`DeviceError::Driver`].
    pub async fn try_call<F, R, E>(&self, f: F) -> Result<R, DeviceError>
    where
        F: FnOnce(&mut D, &CancelToken) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        self.call(f)
            .await?
            .map_err(|e| DeviceError::Driver(e.into()))
    }

    /// Like [`call`](Self::call), from synchronous code on another thread
    /// (such as a camera's capture thread). It must not be used from async
    /// code (tokio panics), nor from the device's own thread (an error).
    pub fn call_blocking<F, R>(&self, f: F) -> Result<R, DeviceError>
    where
        F: FnOnce(&mut D, &CancelToken) -> R + Send + 'static,
        R: Send + 'static,
    {
        if self.shared.thread.get() == Some(&std::thread::current().id()) {
            return Err(DeviceError::Reentrant(self.name().to_owned()));
        }
        let (call, answer) = self.prepare(f)?;
        let sender = self.sender()?;
        self.shared.queued.fetch_add(1, Ordering::Relaxed);
        if sender.blocking_send(Job::Call(call)).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(DeviceError::NotOpen(self.name().to_owned()));
        }
        answer
            .blocking_recv()
            .unwrap_or_else(|_| Err(DeviceError::NotOpen(self.name().to_owned())))
    }

    /// Like [`try_call`](Self::try_call), from synchronous code.
    pub fn try_call_blocking<F, R, E>(&self, f: F) -> Result<R, DeviceError>
    where
        F: FnOnce(&mut D, &CancelToken) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        self.call_blocking(f)?
            .map_err(|e| DeviceError::Driver(e.into()))
    }

    #[allow(clippy::type_complexity)]
    fn prepare<F, R>(
        &self,
        f: F,
    ) -> Result<(Box<dyn Call<D>>, oneshot::Receiver<Result<R, DeviceError>>), DeviceError>
    where
        F: FnOnce(&mut D, &CancelToken) -> R + Send + 'static,
        R: Send + 'static,
    {
        match self.shared.lock().clone() {
            (Some(DeviceState::Faulted), reason) => {
                return Err(DeviceError::Faulted {
                    name: self.name().to_owned(),
                    reason: reason.unwrap_or_default(),
                });
            }
            (Some(DeviceState::Ready | DeviceState::Opening), _) => {}
            _ => return Err(DeviceError::NotOpen(self.name().to_owned())),
        }
        let token =
            InvocationScope::current().map_or_else(CancelToken::new, |s| s.cancel_token().clone());
        let (reply, answer) = oneshot::channel();
        let call = TypedCall {
            f,
            reply,
            token,
            span: tracing::Span::current(),
        };
        Ok((Box::new(call), answer))
    }

    fn sender(&self) -> Result<mpsc::Sender<Job<D>>, DeviceError> {
        lock(&self.sender)
            .clone()
            .ok_or_else(|| DeviceError::NotOpen(self.name().to_owned()))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Creates and opens a driver, catching panics.
fn create<D: Driver>(
    name: &str,
    factory: &mut Factory<D>,
    shared: &Shared,
) -> Result<D, DeviceError> {
    let attempt = catch_unwind(AssertUnwindSafe(|| {
        let mut driver = factory()?;
        driver.open()?;
        Ok::<D, anyhow::Error>(driver)
    }));
    let open_error = |source: anyhow::Error| DeviceError::Open {
        name: name.to_owned(),
        source,
    };
    match attempt {
        Ok(Ok(driver)) => {
            *lock(&shared.abort) = driver.abort_handle();
            shared.set(DeviceState::Ready, None);
            Ok(driver)
        }
        Ok(Err(error)) => {
            shared.set(DeviceState::Closed, None);
            Err(open_error(error))
        }
        Err(payload) => {
            shared.set(DeviceState::Closed, None);
            Err(open_error(anyhow::anyhow!(
                "panicked: {}",
                panic_message(&*payload)
            )))
        }
    }
}

/// Closes a driver, catching panics.
fn close_driver<D: Driver>(driver: &mut D) -> Result<(), anyhow::Error> {
    catch_unwind(AssertUnwindSafe(|| driver.close())).unwrap_or_else(
        |payload: Box<dyn Any + Send>| {
            Err(anyhow::anyhow!("panicked: {}", panic_message(&*payload)))
        },
    )
}

fn device_thread<D: Driver>(
    name: String,
    mut factory: Factory<D>,
    init: Option<Arc<dyn Fn() + Send + Sync>>,
    shared: Arc<Shared>,
    mut mailbox: mpsc::Receiver<Job<D>>,
    opened: oneshot::Sender<Result<(), DeviceError>>,
) {
    let _ = shared.thread.set(std::thread::current().id());
    if let Some(init) = init {
        init();
    }
    let mut driver = match create(&name, &mut factory, &shared) {
        Ok(driver) => {
            let _ = opened.send(Ok(()));
            Some(driver)
        }
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };

    while let Some(job) = mailbox.blocking_recv() {
        match job {
            Job::Call(call) => {
                shared.queued.fetch_sub(1, Ordering::Relaxed);
                if call.abandoned() {
                    continue;
                }
                let (state, reason) = shared.lock().clone();
                match (&mut driver, state) {
                    (Some(d), Some(DeviceState::Ready)) => {
                        if let Some(message) = call.run(d, &name) {
                            tracing::error!(device = %name, "device call panicked: {message}");
                            shared.set(DeviceState::Faulted, Some(message));
                        }
                    }
                    (_, Some(DeviceState::Faulted)) => call.fail(DeviceError::Faulted {
                        name: name.clone(),
                        reason: reason.unwrap_or_default(),
                    }),
                    _ => call.fail(DeviceError::NotOpen(name.clone())),
                }
            }
            Job::Reset(reply) => {
                shared.queued.fetch_sub(1, Ordering::Relaxed);
                if let Some(mut old) = driver.take()
                    && let Err(error) = close_driver(&mut old)
                {
                    tracing::warn!(device = %name, "closing the driver for a reset failed: {error:#}");
                }
                shared.set(DeviceState::Opening, None);
                let result = create(&name, &mut factory, &shared).map(|d| driver = Some(d));
                let _ = reply.send(result);
            }
            Job::Close(reply) => {
                shared.set(DeviceState::Closing, None);
                let result = match driver.as_mut() {
                    Some(d) => close_driver(d).map_err(DeviceError::Driver),
                    None => Ok(()),
                };
                drop(driver.take());
                *lock(&shared.abort) = None;
                shared.set(DeviceState::Closed, None);
                let _ = reply.send(result);
                return;
            }
        }
    }
    // Every sender is gone without a close: close anyway.
    if let Some(mut d) = driver.take() {
        let _ = close_driver(&mut d);
    }
    shared.set(DeviceState::Closed, None);
}

/// The type-erased lifecycle of one device of one Thing.
pub(crate) trait DeviceControl: Send + Sync {
    fn open<'a>(&'a self, name: &'a str) -> crate::BoxFuture<'a, Result<(), DeviceError>>;
    fn close(&self) -> crate::BoxFuture<'_, Result<(), DeviceError>>;
}

pub(crate) struct DeviceAccess<T, D: Driver> {
    pub thing: Arc<T>,
    pub accessor: fn(&T) -> &Device<D>,
}

impl<T: Send + Sync + 'static, D: Driver> DeviceControl for DeviceAccess<T, D> {
    fn open<'a>(&'a self, name: &'a str) -> crate::BoxFuture<'a, Result<(), DeviceError>> {
        Box::pin((self.accessor)(&self.thing).open(name))
    }

    fn close(&self) -> crate::BoxFuture<'_, Result<(), DeviceError>> {
        Box::pin((self.accessor)(&self.thing).close())
    }
}
