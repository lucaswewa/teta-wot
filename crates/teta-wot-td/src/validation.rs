//! Validation against the W3C TD 1.1 JSON Schema (`validation` feature).
//!
//! The schema is vendored in `schema/td-json-schema-validation.json`. It is
//! the W3C-hosted copy at <https://www.w3.org/2022/wot/td-schema/v1.1>,
//! version `1.1-12-March-2025`.

use std::fmt;
use std::sync::LazyLock;

use serde_json::Value;

use crate::ThingDescription;

/// The vendored W3C TD 1.1 JSON Schema, as text.
pub const TD_JSON_SCHEMA: &str = include_str!("../schema/td-json-schema-validation.json");

static VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema: Value =
        serde_json::from_str(TD_JSON_SCHEMA).expect("the vendored TD schema is JSON");
    jsonschema::draft7::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("the vendored TD schema compiles")
});

/// One way in which a TD breaks the W3C JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    /// Where in the TD, as a JSON pointer.
    pub instance_path: String,
    /// Which part of the schema, as a JSON pointer.
    pub schema_path: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for SchemaViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {} (schema {})",
            self.message, self.instance_path, self.schema_path
        )
    }
}

/// A TD doesn't validate against the W3C TD 1.1 JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the TD doesn't match the W3C TD 1.1 JSON Schema:{}", list(.violations))]
pub struct SchemaValidationError {
    /// Every violation found.
    pub violations: Vec<SchemaViolation>,
}

fn list(violations: &[SchemaViolation]) -> String {
    violations.iter().map(|v| format!("\n- {v}")).collect()
}

/// Validates TD JSON against the vendored W3C TD 1.1 JSON Schema.
pub fn validate_json(td: &Value) -> Result<(), SchemaValidationError> {
    let violations: Vec<_> = VALIDATOR
        .iter_errors(td)
        .map(|error| SchemaViolation {
            instance_path: error.instance_path().to_string(),
            schema_path: error.schema_path().to_string(),
            message: error.to_string(),
        })
        .collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(SchemaValidationError { violations })
    }
}

impl ThingDescription {
    /// Serialises the TD and validates it against the vendored W3C TD 1.1
    /// JSON Schema.
    pub fn validate_schema(&self) -> Result<(), SchemaValidationError> {
        let json = serde_json::to_value(self).map_err(|e| SchemaValidationError {
            violations: vec![SchemaViolation {
                instance_path: String::new(),
                schema_path: String::new(),
                message: format!("the TD can't be serialised: {e}"),
            }],
        })?;
        validate_json(&json)
    }
}
