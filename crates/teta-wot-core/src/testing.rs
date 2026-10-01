//! Helpers for testing Things without a server (`testing` feature).

use std::future::Future;
use std::sync::{Arc, Once};

use crate::context::{ActionCtx, InvocationScope};
use crate::lock::GlobalLock;

/// Installs the global `tracing` subscriber with the invocation log layer
/// (see [`crate::logging::init`]), once per process. Tests that check
/// invocation logs need it.
pub fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = crate::logging::init(false);
    });
}

/// An [`ActionCtx`] for a stand-alone invocation, to call an action's
/// function directly. Cancel it with `ctx.cancel_token().cancel()`, and
/// read its log with `ctx.logs()` (after [`init_tracing`]).
pub fn action_ctx(thing: &str) -> ActionCtx {
    ActionCtx::new(InvocationScope::fake(), thing.into(), None)
}

/// Like [`action_ctx`], with a global lock.
pub fn action_ctx_with_lock(thing: &str, lock: Arc<GlobalLock>) -> ActionCtx {
    ActionCtx::new(InvocationScope::fake(), thing.into(), Some(lock))
}

/// Runs `future` in `ctx`'s invocation scope and span, as the runtime runs an
/// action.
pub fn in_invocation<F: Future>(ctx: &ActionCtx, future: F) -> impl Future<Output = F::Output> {
    ctx.scope().clone().run(future)
}
