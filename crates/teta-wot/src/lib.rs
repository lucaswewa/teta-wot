//! Web of Things support for the teta-wot workspace.
//!
//! This facade is the only crate applications depend on. The internal crates
//! are free to change shape behind it.
//!
//! So far it provides:
//!
//! - [`td`]: the Thing Description model, its builders, and the conversion
//!   from Rust types to TD `DataSchema`s.
//! - the runtime: Things defined with [`ThingDefinition`], property cells ([`Prop`]),
//!   actions ([`Action`]) with cancellation, invocations, the global lock, the
//!   message broker and device actors ([`Device`]), all run by a [`Runtime`]
//!
//! `use teta_wot::prelude::*;` brings in what Thing code usually needs.

pub use teta_wot_core::*;
pub use teta_wot_td as td;

/// What Thing code usually nddes.
pub mod prelude {
    pub use teta_wot_core::{
        Action, ActionCtx, ActionError, CancelToken, Cancelled, Constraints, DataProperty, Device,
        DeviceError, DeviceOptions, Driver, FunctionalProperty, InvocationStatus, NoInput, Prop,
        PropertyError, Runtime, Thing, ThingCtx, ThingDefinition, cancellable_sleep,
        check_cancelled,
    };
}
