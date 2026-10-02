//! Runtime core of `wot-rs`.
//!
//! - [`Thing`] and [`ThingDefinition`]: the public builder API that
//!   describes a Thing type's properties, actions, devices and custom
//!   endpoints. The authoring macros (`teta_wot::Thing`, `teta_wot::thing_impl`)
//!   generate it, with the support in `__private`.
//! - [`FromConfig`]: building a Thing from its typed configuration.
//! - [`ThingRef`]: in-process calls between Things, with the same
//!   validation and locking as HTTP requests.
//! - [`slots`]: [`Slot`], [`OptSlot`] and [`SlotMap`] fields connecting
//!   Things to each other, and the start order they imply.
//! - [`settings`]: settings files.
//! - [`Server`] and [`Dep`]: what a Thing sees of its server (other Things,
//!   services, the application configuration, every Thing's state).
//! - [`Prop<T>`], [`DataProperty`] and [`FunctionalProperty`]: property
//!   cells with validation and change notification, and properties backed
//!   by async getters and setters.
//! - [`Event<T>`] and [`EventSpec`]: events a Thing emits, from async or
//!   synchronous code.
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
pub mod config;
pub mod context;
pub mod device;
pub mod endpoint;
pub mod event;
pub mod inprocess;
pub mod invocation;
pub mod lock;
pub mod logging;
pub mod logs;
pub mod problem;
pub mod property;
mod reserved;
pub mod runtime;
pub mod server;
pub mod settings;
pub mod slots;
#[cfg(feature = "testing")]
pub mod testing;
pub mod thing;
pub mod validate;

#[doc(hidden)]
#[path = "macro_support.rs"]
pub mod __private;

pub use action::{Action, ActionError, ActionSpec, DEFAULT_RETENTION, NoInput};
pub use broker::{Message, MessageBroker, MessageKind, Subscription};
pub use cancel::{CancelToken, Cancelled};
pub use config::{FromConfig, NoConfig};
pub use context::{
    ActionCtx, ChildInvocation, HoldLockError, InvocationScope, cancellable_sleep, check_cancelled,
};
pub use device::{AbortHandle, Device, DeviceError, DeviceOptions, DeviceState, Driver};
pub use endpoint::{EndpointEntry, EndpointHandler, EndpointSpec};
pub use event::{Event, EventData, EventEntry, EventSpec};
pub use inprocess::ThingRef;
pub use invocation::{
    CancelError, Invocation, InvocationManager, InvocationRecord, InvocationStatus,
};
pub use lock::{GlobalLock, GlobalLockBusy, GlobalLockGuard};
pub use logs::LogRecord;
pub use problem::ProblemDetails;
pub use property::{
    DataProperty, FunctionalProperty, Prop, PropValue, PropertyEntry, PropertyError, PropertySpec,
};
pub use reserved::{RESERVED_AFFORDANCE_NAMES, affordance_name_problem};
pub use runtime::{
    ActionEntry, BuildError, Runtime, RuntimeBuilder, StartupError, TdOptions, ThingHandle,
};
pub use server::{Dep, Server};
pub use settings::to_settings_json;
pub use slots::{OptSlot, Slot, SlotField, SlotKind, SlotMap, SlotSelection, SlotTarget};
pub use teta_wot_td::Constraints;
pub use thing::{DefinitionError, Thing, ThingCtx, ThingDefinition, split_docstring};
pub use validate::{LocItem, ValidationError, ValidationIssue};

/// A boxed, sendable future: what the methods of a dyn-compatible trait
/// return in place of `async fn` (for `#[teta_wot::interface]` traits).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
