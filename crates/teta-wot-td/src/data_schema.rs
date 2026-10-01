//! TD `DataSchema`: the TD's JSON-Schema-like description of data.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::OneOrMany;

/// Human-readable text in several languages, keyed by language tag
/// (`titles`, `descriptions`).
pub type MultiLanguage = IndexMap<String, String>;

/// The `type` of a [`DataSchema`]. TD 1.1 allows exactly one; a JSON Schema
/// `type` array is converted to `oneOf` (see [`crate::convert`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    /// `true` or `false`.
    Boolean,
    /// A whole number.
    Integer,
    /// Any number.
    Number,
    /// A string.
    String,
    /// A JSON object.
    Object,
    /// A JSON array.
    Array,
    /// `null`.
    Null,
}

impl DataType {
    /// The name used in JSON, for example `"integer"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::Boolean => "boolean",
            DataType::Integer => "integer",
            DataType::Number => "number",
            DataType::String => "string",
            DataType::Object => "object",
            DataType::Array => "array",
            DataType::Null => "null",
        }
    }
}

/// The `items` of an array schema: one schema for every item, or one schema
/// per position (a tuple).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArrayItems {
    /// A schema for each position, as for a Rust tuple.
    Tuple(Vec<DataSchema>),
    /// One schema that every item follows, as for a `Vec`.
    Single(Box<DataSchema>),
}

/// A TD `DataSchema` (TD 1.1 §5.3.2), including the terms of its array,
/// number, integer, object and string subclasses.
///
/// Terms that TD 1.1 doesn't define, such as JSON Schema's
/// `additionalProperties`, are kept in [`extra`](Self::extra) and written
/// after the typed terms.
///
/// A `null` `default` or `const` is never written, and deserialising `null` into
/// an `Option` gives `None` anyway. Use `"type": "null"` to describe a value
/// that must be `null`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSchema {
    /// Semantic annotations (`@type`).
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<OneOrMany<String>>,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Descriptions in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<MultiLanguage>,
    /// Titles in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titles: Option<MultiLanguage>,
    /// Whether the value can only be written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_only: Option<bool>,
    /// Whether the value can only be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// The value matches exactly one of these schemas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_of: Option<Vec<DataSchema>>,
    /// Unit of measure, for example `"degree Celsius"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The value is one of these.
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<Vec<Value>>,
    /// A format such as `date-time` or `uri`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The value is exactly this.
    #[serde(rename = "const", skip_serializing_if = "is_absent")]
    pub constant: Option<Value>,
    /// The value used when none is given.
    #[serde(skip_serializing_if = "is_absent")]
    pub default: Option<Value>,
    /// The JSON type.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub data_type: Option<DataType>,
    /// Array items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<ArrayItems>,
    /// Maximum number of array items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u64>,
    /// Minimum number of array items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u64>,
    /// Inclusive lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Number>,
    /// Inclusive upper bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<Number>,
    /// Exclusive lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_minimum: Option<Number>,
    /// Exclusive upper bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_maximum: Option<Number>,
    /// The value is a multiple of this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_of: Option<Number>,
    /// Object members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<IndexMap<String, DataSchema>>,
    /// Object members that must be present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    /// Minimum string length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    /// Maximum string length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    /// A regular expression the string matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Encoding of the string's content, for example `base64`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<String>,
    /// Media type of the string's content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_media_type: Option<String>,
    /// Terms that TD 1.1 doesn't define for `DataSchema`, such as
    /// `additionalProperties`, `propertyNames` or prefixed extension terms.
    ///
    /// Keys must not repeat a typed term, or it would be written twice.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Whether an optional JSON value should be left out: `None` or `null`.
pub(crate) fn is_absent(value: &Option<Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

impl DataSchema {
    /// A schema with no terms, which accepts any value.
    pub fn any() -> Self {
        Self::default()
    }

    /// A schema for one JSON type, with no other terms.
    pub fn of(data_type: DataType) -> Self {
        Self {
            data_type: Some(data_type),
            ..Self::default()
        }
    }

    /// Sets the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the unit of measure.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Sets the default value. A `null` default is not written.
    pub fn with_default(mut self, default: Value) -> Self {
        self.default = Some(default);
        self
    }

    /// Adds a semantic annotation (`@type`), such as `saref:Temperature`.
    pub fn with_semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        push_semantic_type(&mut self.semantic_type, semantic_type.into());
        self
    }
}

/// Adds `value` to an `@type` term, keeping a single annotation as a string.
pub(crate) fn push_semantic_type(target: &mut Option<OneOrMany<String>>, value: String) {
    *target = Some(match target.take() {
        None => OneOrMany::One(value),
        Some(OneOrMany::One(first)) => OneOrMany::Many(vec![first, value]),
        Some(OneOrMany::Many(mut all)) => {
            all.push(value);
            OneOrMany::Many(all)
        }
    });
}
