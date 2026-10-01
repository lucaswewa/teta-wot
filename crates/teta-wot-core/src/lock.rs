//! The optional global lock.
//!
//! When enabled, property writes and action invocations take the lock, so
//! they happen one at a time:
//!
//! - acquiring waits at most a short timeout (50 ms by default) and then
//!   fails with [`GlobalLockBusy`];
//! - it is reentrant, but per *invocation* rather than per thread: the
//!   owner is the root invocation, so an action that calls another action
//!   or writes a property in-process, or a child invocation, doesn't block
//!   on the lock its own invocation holds.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use uuid::Uuid;

/// Default time to wait for the global lock.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_millis(50);

/// The global lock is held by someone else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("The global lock could not be acquired.")]
pub struct GlobalLockBusy;

/// Who holds the lock, and how many times.
#[derive(Debug, Clone, Copy)]
struct Held {
    owner: Uuid,
    depth: usize,
}

/// A reentrant lock with an acquisition timeout, owned by invocations.
#[derive(Debug)]
pub struct GlobalLock {
    held: Mutex<Option<Held>>,
    released: Notify,
    timeout: Duration,
}

impl GlobalLock {
    /// A free lock with the given acquisition timeout.
    pub fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(None),
            released: Notify::new(),
            timeout,
        })
    }

    /// How long [`acquire`](Self::acquire) waits.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The current owner, if the lock is held.
    pub fn owner(&self) -> Option<Uuid> {
        self.held().map(|h| h.owner)
    }

    fn held(&self) -> Option<Held> {
        *self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the lock for `owner` if it is free or already `owner`'s.
    pub fn try_acquire(self: &Arc<Self>, owner: Uuid) -> Result<GlobalLockGuard, GlobalLockBusy> {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        match &mut *held {
            None => *held = Some(Held { owner, depth: 1 }),
            Some(h) if h.owner == owner => h.depth += 1,
            Some(_) => return Err(GlobalLockBusy),
        }
        Ok(GlobalLockGuard {
            lock: Arc::clone(self),
        })
    }

    /// Takes the lock for `owner`, waiting up to the timeout for it to be
    /// released.
    pub async fn acquire(self: &Arc<Self>, owner: Uuid) -> Result<GlobalLockGuard, GlobalLockBusy> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            if let Ok(guard) = self.try_acquire(owner) {
                return Ok(guard);
            }
            if tokio::time::timeout_at(deadline, released).await.is_err() {
                return self.try_acquire(owner);
            }
        }
    }

    fn release(&self) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = &mut *held {
            h.depth -= 1;
            if h.depth == 0 {
                *held = None;
                drop(held);
                self.released.notify_waiters();
            }
        }
    }
}

/// Holds the global lock until dropped.
#[derive(Debug)]
#[must_use = "the lock is released when the guard is dropped"]
pub struct GlobalLockGuard {
    lock: Arc<GlobalLock>,
}

impl Drop for GlobalLockGuard {
    fn drop(&mut self) {
        self.lock.release();
    }
}
