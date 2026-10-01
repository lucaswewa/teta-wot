//! Cooperative cancellation.
//!
//! Every invocation has a [`CancelToken`]. Cancelling an invocation (for
//! example with `DELETE /action_invocations/{id}`) sets the token; action
//! code notices at its next [`check`](CancelToken::check), cancellable
//! [`sleep`](CancelToken::sleep) or [`cancelled`](CancelToken::cancelled).
//!
//! Observing a cancellation *consumes* it `raise_if_set` does: the flag is
//! cleared and [`Cancelled`] is returned. An action that handles `Cancelled`
//! can therefore carry on, and a later cancellation is seen again. Futures
//! are never aborted.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::Notify;

/// The invocation was cancelled.
///
/// Return it from an action (usually with `?`) to end the invocation with
/// the status `cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, thiserror::Error)]
#[error("The action was cancelled.")]
pub struct Cancelled;

#[derive(Debug, Default)]
struct State {
    flag: Mutex<bool>,
    condvar: Condvar,
    notify: Notify,
}

impl State {
    fn flag(&self) -> MutexGuard<'_, bool> {
        // The flag is a plain bool, so a poisoned lock still holds a valid value.
        self.flag.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A resettable cancellation flag, usable from async and blocking code.
///
/// Clones share the flag. The invocation's token is passed into device
/// closures, so synchronous driver code can check it too.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    state: Arc<State>,
}

impl CancelToken {
    /// A new token that isn't cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation and wakes everything waiting on the token.
    pub fn cancel(&self) {
        *self.state.flag() = true;
        self.state.condvar.notify_all();
        self.state.notify.notify_waiters();
    }

    /// Whether cancellation has been requested and not yet observed.
    /// This does not consume it.
    pub fn is_cancelled(&self) -> bool {
        *self.state.flag()
    }

    /// Consumes a pending cancellation: returns `Err(Cancelled)` and clears
    /// the flag if one was requested.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.take() { Err(Cancelled) } else { Ok(()) }
    }

    fn take(&self) -> bool {
        std::mem::take(&mut *self.state.flag())
    }

    /// Waits until cancellation is requested, then consumes it.
    ///
    /// Useful in `tokio::select!` to stop other work when the invocation is
    /// cancelled.
    pub async fn cancelled(&self) -> Cancelled {
        loop {
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            // Register for wake-ups before checking, so none is missed.
            notified.as_mut().enable();
            if self.take() {
                return Cancelled;
            }
            notified.await;
        }
    }

    /// Sleeps for `duration`, unless cancellation is requested first, in
    /// which case it is consumed and `Err(Cancelled)` is returned.
    pub async fn sleep(&self, duration: Duration) -> Result<(), Cancelled> {
        tokio::select! {
            biased;
            cancelled = self.cancelled() => Err(cancelled),
            () = tokio::time::sleep(duration) => Ok(()),
        }
    }

    /// Blocks the current thread like [`std::thread::sleep`], but returns
    /// early with `Err(Cancelled)` (consuming it) if cancellation is
    /// requested. For driver code on a device thread.
    pub fn sleep_blocking(&self, duration: Duration) -> Result<(), Cancelled> {
        let guard = self.state.flag();
        let (mut guard, _) = self
            .state
            .condvar
            .wait_timeout_while(guard, duration, |set| !*set)
            .unwrap_or_else(|e| e.into_inner());
        if std::mem::take(&mut *guard) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}
