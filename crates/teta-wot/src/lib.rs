//! Web of Things support for the teta-wot workspace.
//!
//! This facade is the only crate applications depend on. The internal crates
//! are free to change shape behind it.
//!
//! So far it provides:
//!
//! - [`td`]: the Thing Description model, its builders, and the conversion
//!   from Rust types to TD `DataSchema`s.

pub use teta_wot_td as td;
