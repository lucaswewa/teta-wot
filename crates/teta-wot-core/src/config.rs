//! Constructing Things from their configuration.
//!
//! `#[derive(Thing)]` implements [`FromConfig`]: the Thing's typed
//! configuration is deserialised from the `kwargs` of its entry in the
//! configuration file (Phase 5), or from any JSON value, and the macro builds
//! the Thing from it.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::thing::Thing;

/// A Thing that can be built from a typed configuration.
pub trait FromConfig: Thing {
    /// The configuration, deserialised from the Thing's `kwargs`.
    type Config: DeserializeOwned + Send + Sync + 'static;

    /// Builds the Thing.
    fn from_config(config: Self::Config) -> Self;
}

/// The configuration of a Thing that takes none: `{}` or `null`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct NoConfig {}

impl<'de> Deserialize<'de> for NoConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Empty {}

        Option::<Empty>::deserialize(deserializer).map(|_| NoConfig {})
    }
}
