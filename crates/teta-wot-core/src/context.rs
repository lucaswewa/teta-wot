//! The invocation context: which invocation the current code belongs to.
//!
//! Every invocation runs with an [`InvocationScope`] in a tokio task-local
//! and inside a `tracing` span named `invocation`. That is how
//! [`cancellable_sleep`] and [`check_cancelled`] find the invocation's
//! cancel token, how the global lock recognises its owner, and how log
//! events reach the invocation's log. Actions get the same things
//! explicitly through [`ActionCtx`].

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::Instrument;
use uuid::Uuid;

use crate::action::ActionError;
use crate::cancel::{CancelToken, Cancelled};
use crate::lock::{GlobalLock, GlobalLockBusy, GlobalLockGuard};
use crate::logs::{LogBuffer, LogRecord};
use crate::server::Server;

tokio::task_local! {
    static SCOPE: InvocationScope;
}

/// The identity, cancel token and log of the invocation that code runs for.
#[derive(Debug, Clone)]
pub struct InvocationScope {
    id: Uuid,
    lock_owner: Uuid,
    cancel: CancelToken,
    logs: Arc<LogBuffer>,
}

impl InvocationScope {
    pub(crate) fn new(
        id: Uuid,
        lock_owner: Uuid,
        cancel: CancelToken,
        logs: Arc<LogBuffer>,
    ) -> Self {
        Self {
            id,
            lock_owner,
            cancel,
            logs,
        }
    }

    /// A scope for a new, stand-alone invocation, for running action code
    /// in tests.
    pub fn fake() -> Self {
        let id = Uuid::new_v4();
        Self::new(id, id, CancelToken::new(), LogBuffer::register(id))
    }

    /// The scope of the code that is running, if it is part of an invocation.
    pub fn current() -> Option<Self> {
        SCOPE.try_with(Clone::clone).ok()
    }

    /// The invocation's ID.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Who owns the global lock on this invocation's behalf: the root
    /// invocation, so child invocations share their parent's ownership.
    pub fn lock_owner(&self) -> Uuid {
        self.lock_owner
    }

    /// The invocation's cancel token.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// The invocation's log so far.
    pub fn logs(&self) -> Vec<LogRecord> {
        self.logs.records()
    }

    /// Runs `future` in this scope, inside a new `invocation` span (child of
    /// the current span).
    pub fn run<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        let span = tracing::info_span!("invocation", invocation_id = %self.id);
        SCOPE.scope(self, future.instrument(span))
    }

    /// Runs blocking code in this scope (the caller should already be in the
    /// invocation's span).
    pub fn run_blocking<R>(self, f: impl FnOnce() -> R) -> R {
        SCOPE.sync_scope(self, f)
    }

    fn child(&self) -> Self {
        let id = Uuid::new_v4();
        Self::new(
            id,
            self.lock_owner,
            CancelToken::new(),
            LogBuffer::register(id),
        )
    }
}

/// Sleeps for `duration`, returning early with `Err(Cancelled)` if the
/// current invocation is cancelled. Outside an invocation it is a plain sleep.
pub async fn cancellable_sleep(duration: Duration) -> Result<(), Cancelled> {
    match InvocationScope::current() {
        Some(scope) => scope.cancel.sleep(duration).await,
        None => {
            tokio::time::sleep(duration).await;
            Ok(())
        }
    }
}

/// Returns `Err(Cancelled)` if the current invocation has been cancelled,
/// consuming the cancellation. Outside an invocation it does nothing.
pub fn check_cancelled() -> Result<(), Cancelled> {
    InvocationScope::current().map_or(Ok(()), |scope| scope.cancel.check())
}

/// The global lock can't be held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HoldLockError {
    /// The server doesn't have a global lock.
    #[error("The global lock is required, but is not enabled.")]
    NotEnabled,
    /// Someone else holds it.
    #[error(transparent)]
    Busy(#[from] GlobalLockBusy),
}

/// What an action gets besides its input: its invocation's identity,
/// cancellation, log and the global lock and the server.
#[derive(Debug, Clone)]
pub struct ActionCtx {
    scope: InvocationScope,
    thing: Arc<str>,
    lock: Option<Arc<GlobalLock>>,
    server: Server,
}

impl ActionCtx {
    pub(crate) fn new(
        scope: InvocationScope,
        thing: Arc<str>,
        lock: Option<Arc<GlobalLock>>,
        server: Server,
    ) -> Self {
        Self {
            scope,
            thing,
            lock,
            server,
        }
    }

    /// The server: other Things, services, the application configuration
    /// and the state of every Thing.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The invocation's ID.
    pub fn invocation_id(&self) -> Uuid {
        self.scope.id
    }

    /// The name of the Thing the action belongs to.
    pub fn thing_name(&self) -> &str {
        &self.thing
    }

    /// The invocation's scope.
    pub fn scope(&self) -> &InvocationScope {
        &self.scope
    }

    /// The invocation's cancel token, for code that can't use the methods
    /// below (for example a device closure receives it directly).
    pub fn cancel_token(&self) -> &CancelToken {
        &self.scope.cancel
    }

    /// Sleeps, returning early with `Err(Cancelled)` if the invocation is
    /// cancelled.
    pub async fn sleep(&self, duration: Duration) -> Result<(), Cancelled> {
        self.scope.cancel.sleep(duration).await
    }

    /// Returns `Err(Cancelled)` if the invocation has been cancelled,
    /// consuming the cancellation.
    pub fn check_cancelled(&self) -> Result<(), Cancelled> {
        self.scope.cancel.check()
    }

    /// Waits until the invocation is cancelled (for `tokio::select!`),
    /// consuming the cancellation.
    pub async fn cancelled(&self) -> Cancelled {
        self.scope.cancel.cancelled().await
    }

    /// The invocation's log so far.
    pub fn logs(&self) -> Vec<LogRecord> {
        self.scope.logs()
    }

    /// Holds the global lock until the guard is dropped, for actions that
    /// opted out of holding it automatically.
    pub async fn hold_global_lock(&self) -> Result<GlobalLockGuard, HoldLockError> {
        let lock = self.lock.as_ref().ok_or(HoldLockError::NotEnabled)?;
        Ok(lock.acquire(self.scope.lock_owner).await?)
    }

    /// Runs `f` as a child invocation in a new task, with its own ID, cancel
    /// token and log. It shares this invocation's global-lock ownership.
    pub fn spawn_child<F, Fut, R>(&self, f: F) -> ChildInvocation<R>
    where
        F: FnOnce(ActionCtx) -> Fut,
        Fut: Future<Output = Result<R, ActionError>> + Send + 'static,
        R: Send + 'static,
    {
        let scope = self.scope.child();
        let child = ActionCtx::new(
            scope.clone(),
            Arc::clone(&self.thing),
            self.lock.clone(),
            self.server.clone(),
        );
        let id = scope.id;
        let cancel = scope.cancel.clone();
        let logs = Arc::clone(&scope.logs);
        let handle = tokio::spawn(scope.run(f(child)));
        ChildInvocation {
            id,
            cancel,
            logs,
            parent: self.scope.cancel.clone(),
            handle,
        }
    }

    /// Runs blocking code on tokio's blocking pool, in this invocation's
    /// context: its log captures events, and [`check_cancelled`] works. The
    /// closure also gets the cancel token. A panic in `f` resumes here.
    pub async fn blocking<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&CancelToken) -> R + Send + 'static,
        R: Send + 'static,
    {
        let scope = self.scope.clone();
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let token = scope.cancel.clone();
            scope.run_blocking(|| f(&token))
        })
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
    }
}

/// A child invocation started with [`ActionCtx::spawn_child`].
#[derive(Debug)]
#[must_use = "a child invocation should be joined"]
pub struct ChildInvocation<R> {
    id: Uuid,
    cancel: CancelToken,
    logs: Arc<LogBuffer>,
    parent: CancelToken,
    handle: JoinHandle<Result<R, ActionError>>,
}

impl<R> ChildInvocation<R> {
    /// The child's invocation ID.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Asks the child to stop.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The child's log so far.
    pub fn logs(&self) -> Vec<LogRecord> {
        self.logs.records()
    }

    /// Waits for the child, passing on cancellation: each time the parent is
    /// cancelled while waiting, the cancellation is consumed and the child is
    /// cancelled. If that happened at least once, the result is `Err(Cancelled)`
    /// once the child has finished; otherwise it is the child's own result.
    pub async fn join(mut self) -> Result<R, ActionError> {
        let mut parent_cancelled = false;
        loop {
            tokio::select! {
                result = &mut self.handle => {
                    let result = result.unwrap_or_else(|e| Err(ActionError::from_join(e)));
                    return if parent_cancelled { Err(Cancelled.into()) } else { result };
                }
                _ = self.parent.cancelled() => {
                    parent_cancelled = true;
                    self.cancel.cancel();
                }
            }
        }
    }
}

/// Polls a future, turning a panic into an `Err` with the panic payload.
pub(crate) struct CatchUnwind<F>(pub(crate) F);

impl<F: Future + Unpin> Future for CatchUnwind<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = &mut self.get_mut().0;
        match std::panic::catch_unwind(AssertUnwindSafe(|| Pin::new(&mut *inner).poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

/// The message of a panic payload.
pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}
