//! Thing Description support for the teta-wot workspace.
//!
//! This crate has no async or server dependencies. It provides:
//!
//! - a serde model of TD 1.1: [`ThingDescription`], the affordances,
//!   [`DataSchema`], [`Form`], [`Link`], [`SecurityScheme`] and the
//!   `@context` ([`Context`]);
//! - builders that check the TD rules the type system can't
//!   ([`ThingDescription::builder`], [`PropertyAffordance::builder`], ..);
//! - conversion from Rust types (via [Schemars]) and JSON Schema to
//!   [`DataSchema`] ([`DataSchema::for_type`], [`convert`]);
//! - Value [`Constraints`] and how they map into a schema;
//! - with the `validation` feature, validation against the vendored W3C TD
//!   1.1 JSON Schema ([`validation`]).
//!
//! ```
//! use teta_wot_td::{ActionAffordance, DataSchema, Form, Operation, PropertyAffordance, ThingDescription};
//!
//! let td = ThingDescription::builder("Counter")
//!     .base("http://localhost:5000/")
//!     .property(
//!         "count",
//!         PropertyAffordance::builder(DataSchema::for_type::<i64>()?)
//!             .title("The current count")
//!             .read_only(true)
//!             .form(Form::new("/counter/count").with_op([Operation::ReadProperty])),
//!     )
//!     .action(
//!         "increment",
//!         ActionAffordance::builder()
//!             .output(DataSchema::for_type::<()>()?)
//!             .form(Form::new("/counter/increment").with_op([Operation::InvokeAction])),
//!     )
//!     .build()?;
//!
//! assert_eq!(td.security_definitions.keys().next().unwrap(), "no_security");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod affordance;
mod constraints;
mod context;
pub mod convert;
mod data_schema;
mod error;
mod form;
mod link;
mod one_or_many;
mod security;
mod thing;
#[cfg(feature = "validation")]
pub mod validation;

pub use schemars;

pub use affordance::{
    ActionAffordance, ActionBuilder, EventAffordance, EventBuilder, PropertyAffordance,
    PropertyBuilder,
};
pub use constraints::{Bound, ConstraintError, Constraints};
pub use context::{Context, ContextEntry, TD_CONTEXT_V1, TD_CONTEXT_V1_1};
pub use convert::ConversionError;
pub use data_schema::{ArrayItems, DataSchema, DataType, MultiLanguage};
pub use error::TdError;
pub use form::{AdditionalExpectedResponse, ExpectedResponse, Form, Operation, OperationScope};
pub use link::Link;
pub use one_or_many::OneOrMany;
pub use security::{CredentialLocation, SecurityScheme, SecuritySchemeType};
pub use thing::{NO_SECURITY_NAME, ThingBuilder, ThingDescription, VersionInfo};
