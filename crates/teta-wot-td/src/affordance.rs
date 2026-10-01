//! Interaction affordances: properties, actions and events.

use indexmap::IndexMap;
use serde::ser::{Error as _, SerializeMap};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::data_schema::{MultiLanguage, push_semantic_type};
use crate::{DataSchema, Form, OneOrMany};

/// A TD `PropertyAffordance` (TD 1.1 §5.3.1.2).
///
/// A property affordance *is* a data schema, so its data-schema terms
/// (including `title` and `description`) live in [`schema`](Self::schema).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyAffordance {
    /// The property's data schema, title and description.
    #[serde(flatten)]
    pub schema: DataSchema,
    /// How to read, write and observe the property. A TD needs at least one.
    pub forms: Vec<Form>,
    /// Schemas of the URI template variables used in the forms.
    #[serde(default)]
    pub uri_variables: Option<IndexMap<String, DataSchema>>,
    /// Whether the property can be observed.
    #[serde(default)]
    pub observable: Option<bool>,
}

impl Serialize for PropertyAffordance {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Value::Object(schema) = serde_json::to_value(&self.schema).map_err(S::Error::custom)?
        else {
            return Err(S::Error::custom("a DataSchema must serialise to an object"));
        };
        let mut map = serializer.serialize_map(None)?;
        for (key, value) in schema
            .iter()
            .filter(|(k, _)| !self.schema.extra.contains_key(*k))
        {
            map.serialize_entry(key, value)?;
        }
        map.serialize_entry("forms", &self.forms)?;
        if let Some(uri_variables) = &self.uri_variables {
            map.serialize_entry("uriVariables", uri_variables)?;
        }
        if let Some(observable) = &self.observable {
            map.serialize_entry("observable", observable)?;
        }
        for (key, value) in &self.schema.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// A TD `ActionAffordance` (TD 1.1 §5.3.1.3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionAffordance {
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Descriptions in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<MultiLanguage>,
    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Titles in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titles: Option<MultiLanguage>,
    /// How to invoke (and query or cancel) the action. A TD needs at least one.
    pub forms: Vec<Form>,
    /// Schemas of the URI template variables used in the forms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri_variables: Option<IndexMap<String, DataSchema>>,
    /// Semantic annotations.
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<OneOrMany<String>>,
    /// Schema of the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<DataSchema>,
    /// Schema of the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<DataSchema>,
    /// Whether invoking the action changes no state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe: Option<bool>,
    /// Whether invoking the action twice has the same effect as once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    /// Whether the response carries the output (`true`) or the invocation
    /// continues after the response (`false`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synchronous: Option<bool>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A TD `EventAffordance` (TD 1.1 §5.3.1.4). Fields follow the same pattern
/// as [`ActionAffordance`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAffordance {
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Descriptions in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<MultiLanguage>,
    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Titles in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titles: Option<MultiLanguage>,
    /// How to subscribe and unsubscribe. A TD needs at least one.
    pub forms: Vec<Form>,
    /// Schemas of the URI template variables used in the forms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri_variables: Option<IndexMap<String, DataSchema>>,
    /// Semantic annotations.
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<OneOrMany<String>>,
    /// Data sent when subscribing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<DataSchema>,
    /// Data carried by each event notification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<DataSchema>,
    /// Data the consumer sends back in response to a notification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_response: Option<DataSchema>,
    /// Data sent when unsubscribing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<DataSchema>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl PropertyAffordance {
    /// Starts building a property with the given data schema.
    ///
    /// The builder always writes `readOnly` and `writeOnly`; both start as `false`.
    pub fn builder(schema: DataSchema) -> PropertyBuilder {
        PropertyBuilder {
            property: PropertyAffordance {
                schema: DataSchema {
                    read_only: Some(false),
                    write_only: Some(false),
                    ..schema
                },
                ..Default::default()
            },
        }
    }
}

impl ActionAffordance {
    /// Starts building an action.
    pub fn builder() -> ActionBuilder {
        ActionBuilder {
            action: ActionAffordance::default(),
        }
    }
}

impl EventAffordance {
    /// Starts building an event.
    pub fn builder() -> EventBuilder {
        EventBuilder {
            event: EventAffordance::default(),
        }
    }
}

/// Builds a [`PropertyAffordance`]. Converts into one with `.into()` or [`build`](Self::build).
#[derive(Debug, Clone)]
#[must_use]
pub struct PropertyBuilder {
    property: PropertyAffordance,
}

impl PropertyBuilder {
    /// Sets the title, replacing any title from the data schema.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.property.schema.title = Some(title.into());
        self
    }

    /// Sets the description, replacing any description from the data schema.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.property.schema.description = Some(description.into());
        self
    }

    /// Sets whether the property is read-only.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.property.schema.read_only = Some(read_only);
        self
    }

    /// Sets whether the property is write-only.
    pub fn write_only(mut self, write_only: bool) -> Self {
        self.property.schema.write_only = Some(write_only);
        self
    }

    /// Sets whether the property is observable.
    pub fn observable(mut self, observable: bool) -> Self {
        self.property.observable = Some(observable);
        self
    }

    /// Sets the default value. A `null` default is not written.
    pub fn default_value(mut self, default: Value) -> Self {
        self.property.schema.default = Some(default);
        self
    }

    /// Sets the unit of measure.
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.property.schema.unit = Some(unit.into());
        self
    }

    /// Adds a semantic annotation (`@type`).
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        push_semantic_type(
            &mut self.property.schema.semantic_type,
            semantic_type.into(),
        );
        self
    }

    /// Adds a form.
    pub fn form(mut self, form: Form) -> Self {
        self.property.forms.push(form);
        self
    }

    /// Declares a URI template variable.
    pub fn uri_variable(mut self, name: impl Into<String>, schema: DataSchema) -> Self {
        self.property
            .uri_variables
            .get_or_insert_with(IndexMap::new)
            .insert(name.into(), schema);
        self
    }

    /// Finishes the property.
    pub fn build(self) -> PropertyAffordance {
        self.property
    }
}

impl From<PropertyBuilder> for PropertyAffordance {
    fn from(builder: PropertyBuilder) -> Self {
        builder.build()
    }
}

/// Builds an [`ActionAffordance`]. Converts into one with `.into()` or [`build`](Self::build).
#[derive(Debug, Clone)]
#[must_use]
pub struct ActionBuilder {
    action: ActionAffordance,
}

impl ActionBuilder {
    /// Sets the title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.action.title = Some(title.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.action.description = Some(description.into());
        self
    }

    /// Sets the input schema.
    pub fn input(mut self, input: DataSchema) -> Self {
        self.action.input = Some(input);
        self
    }

    /// Sets the output schema.
    pub fn output(mut self, output: DataSchema) -> Self {
        self.action.output = Some(output);
        self
    }

    /// Sets whether the action is safe.
    pub fn safe(mut self, safe: bool) -> Self {
        self.action.safe = Some(safe);
        self
    }

    /// Sets whether the action is idempotent.
    pub fn idempotent(mut self, idempotent: bool) -> Self {
        self.action.idempotent = Some(idempotent);
        self
    }

    /// Sets whether the action is synchronous.
    pub fn synchronous(mut self, synchronous: bool) -> Self {
        self.action.synchronous = Some(synchronous);
        self
    }

    /// Adds a semantic annotation (`@type`).
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        push_semantic_type(&mut self.action.semantic_type, semantic_type.into());
        self
    }

    /// Adds a form.
    pub fn form(mut self, form: Form) -> Self {
        self.action.forms.push(form);
        self
    }

    /// Declares a URI template variable.
    pub fn uri_variable(mut self, name: impl Into<String>, schema: DataSchema) -> Self {
        self.action
            .uri_variables
            .get_or_insert_with(IndexMap::new)
            .insert(name.into(), schema);
        self
    }

    /// Finishes the action.
    pub fn build(self) -> ActionAffordance {
        self.action
    }
}

impl From<ActionBuilder> for ActionAffordance {
    fn from(builder: ActionBuilder) -> Self {
        builder.build()
    }
}

/// Builds an [`EventAffordance`]. Converts into one with `.into()` or [`build`](Self::build).
#[derive(Debug, Clone)]
#[must_use]
pub struct EventBuilder {
    event: EventAffordance,
}

impl EventBuilder {
    /// Sets the title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.event.title = Some(title.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.event.description = Some(description.into());
        self
    }

    /// Sets the schema of the notification data.
    pub fn data(mut self, data: DataSchema) -> Self {
        self.event.data = Some(data);
        self
    }

    /// Sets the schema of the data sent when subscribing.
    pub fn subscription(mut self, subscription: DataSchema) -> Self {
        self.event.subscription = Some(subscription);
        self
    }

    /// Sets the schema of the data sent when unsubscribing.
    pub fn cancellation(mut self, cancellation: DataSchema) -> Self {
        self.event.cancellation = Some(cancellation);
        self
    }

    /// Adds a semantic annotation (`@type`).
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        push_semantic_type(&mut self.event.semantic_type, semantic_type.into());
        self
    }

    /// Adds a form.
    pub fn form(mut self, form: Form) -> Self {
        self.event.forms.push(form);
        self
    }

    /// Finishes the event.
    pub fn build(self) -> EventAffordance {
        self.event
    }
}

impl From<EventBuilder> for EventAffordance {
    fn from(builder: EventBuilder) -> Self {
        builder.build()
    }
}
