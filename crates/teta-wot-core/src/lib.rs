//! Runtime core of `teta-wot-core`.
//!
//! - [`Thing`] and [`ThingDefinition`]: the public builder API that
//!   describes a Thing type's properties, actions and devices.
//! - [`Prop<T>`], [`DataProperty`] and [`FunctionalProperty`]: property
//!   cells with validation and change notification, and properties backed
//!   by async getters and setters.
//! - [`Action`], [`ActionCtx`] and [`ActionError`]: async actions with
//!   cooperative cancellation ([`CancelToken`]), child invocations and a
//!   blocking-code helper.
//! - [`Runtime`]: the registry of running Things ([`ThingHandle`]),
//!   invocations ([`InvocationManager`]), the [`MessageBroker`], the
//!   optional [`GlobalLock`], and ordered start and stop.
//! - [`Device<D>`]: synchronous [`Driver`]s on their own threads.
//! - [`validate`]: pydantic-compatible validation of client values.
//! - [`logs`]: invocation logs captured from `tracing`.

use std::future::Future;
use std::pin::Pin;

pub mod action;
pub mod broker;
pub mod cancel;
pub mod context;
pub mod device;
pub mod invocation;
pub mod lock;
pub mod logging;
pub mod logs;
pub mod problem;
pub mod property;
pub mod runtime;
#[cfg(feature = "testing")]
pub mod testing;
pub mod thing;
pub mod validate;

pub use action::{Action, ActionError, ActionSpec, DEFAULT_RETENTION, NoInput};
pub use broker::{Message, MessageBroker, MessageKind, Subscription};
pub use cancel::{CancelToken, Cancelled};
pub use context::{
    ActionCtx, ChildInvocation, HoldLockError, InvocationScope, cancellable_sleep, check_cancelled,
};
pub use device::{AbortHandle, Device, DeviceError, DeviceOptions, DeviceState, Driver};
pub use invocation::{
    CancelError, Invocation, InvocationManager, InvocationRecord, InvocationStatus,
};
pub use lock::{GlobalLock, GlobalLockBusy, GlobalLockGuard};
pub use logs::LogRecord;
pub use problem::ProblemDetails;
pub use property::{
    DataProperty, FunctionalProperty, Prop, PropValue, PropertyEntry, PropertyError, PropertySpec,
};
pub use runtime::{
    ActionEntry, BuildError, Runtime, RuntimeBuilder, StartupError, TdOptions, ThingHandle,
};
pub use teta_wot_td::Constraints;
pub use thing::{DefinitionError, Thing, ThingCtx, ThingDefinition, split_docstring};
pub use validate::{LocItem, ValidationError, ValidationIssue};

/// A boxed, sendable future.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
