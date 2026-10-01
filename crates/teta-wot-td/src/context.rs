//! The TD `@context`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::TdError;

/// The context URI of TD 1.1. Every TD this crate builds uses it.
pub const TD_CONTEXT_V1_1: &str = "https://www.w3.org/2022/wot/td/v1.1";

/// The context URI of TD 1.0. A TD 1.1 may list it only *before* the 1.1 URI.
pub const TD_CONTEXT_V1: &str = "https://www.w3.org/2019/wot/td/v1";

/// One entry of an `@context` array: a context URI, or a map of prefixes
/// (or keywords such as `@language`) to URIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContextEntry {
    /// A context URI.
    Uri(String),
    /// Prefix definitions, for example `{"saref": "https://saref.etsi.org/core/"}`.
    Map(IndexMap<String, String>),
}

impl From<&str> for ContextEntry {
    fn from(uri: &str) -> Self {
        ContextEntry::Uri(uri.to_owned())
    }
}

/// The `@context` of a Thing Description.
///
/// The default is the plain TD 1.1 URI. Adding entries (for semantic annotations)
/// turns it into an array that starts with the TD 1.1 URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Context {
    /// A single context URI.
    Uri(String),
    /// Several entries.
    Array(Vec<ContextEntry>),
}

impl Default for Context {
    fn default() -> Self {
        Context::Uri(TD_CONTEXT_V1_1.to_owned())
    }
}

impl Context {
    /// Appends an entry, converting a single URI into an array first.
    pub fn push(&mut self, entry: ContextEntry) {
        if let Context::Uri(uri) = self {
            *self = Context::Array(vec![ContextEntry::Uri(std::mem::take(uri))]);
        }
        if let Context::Array(entries) = self {
            entries.push(entry);
        }
    }

    /// Adds a prefix definition, such as `saref` → `https://saref.etsi.org/core/`.
    ///
    /// Prefixes are collected in a single map entry after the context URIs.
    pub fn add_prefix(&mut self, prefix: impl Into<String>, uri: impl Into<String>) {
        if let Context::Array(entries) = self
            && let Some(ContextEntry::Map(map)) = entries.last_mut()
        {
            map.insert(prefix.into(), uri.into());
            return;
        }
        self.push(ContextEntry::Map(IndexMap::from([(
            prefix.into(),
            uri.into(),
        )])));
    }

    /// Checks the rules for the `@context` of a TD 1.1 document.
    ///
    /// A context with only the TD 1.0 URI is rejected, because every TD built here is a TD 1.1.
    ///
    /// - A single URI must be the TD 1.1 URI.
    /// - An array must be non-empty and start with the TD 1.1 URI, with no
    ///   TD 1.0 URI after it; or start with the TD 1.0 URI immediately
    ///   followed by the TD 1.1 URI.
    pub fn validate(&self) -> Result<(), TdError> {
        let invalid = |reason: &str| Err(TdError::InvalidContext(reason.to_owned()));
        let entries = match self {
            Context::Uri(uri) if uri == TD_CONTEXT_V1_1 => return Ok(()),
            Context::Uri(uri) => return invalid(&format!("{uri} must be {TD_CONTEXT_V1_1}")),
            Context::Array(entries) => entries,
        };
        let is = |entry: Option<&ContextEntry>, uri: &str| matches!(entry, Some(ContextEntry::Uri(u)) if u == uri);
        if entries.is_empty() {
            invalid("the context can't be an empty array")
        } else if is(entries.first(), TD_CONTEXT_V1_1) {
            if entries[1..].iter().any(|e| is(Some(e), TD_CONTEXT_V1)) {
                invalid("the TD 1.0 context URI is given after the TD 1.1 one")
            } else {
                Ok(())
            }
        } else if is(entries.first(), TD_CONTEXT_V1) {
            if is(entries.get(1), TD_CONTEXT_V1_1) {
                if entries[2..].iter().any(|e| is(Some(e), TD_CONTEXT_V1)) {
                    invalid("the TD 1.0 context URI is given after the TD 1.1 one")
                } else {
                    Ok(())
                }
            } else {
                invalid("the TD 1.0 context URI must be followed by the TD 1.1 one")
            }
        } else {
            invalid(&format!(
                "the context must start with {TD_CONTEXT_V1_1}, or with {TD_CONTEXT_V1} followed by it"
            ))
        }
    }
}
