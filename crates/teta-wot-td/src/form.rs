//! Forms define how to perform an operation on an affordance.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::one_or_many::OneOrMany;

/// Operation scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationScope {
    /// In the forms of a property affordance.
    Property,
    /// In the forms of an action affordance.
    Action,
    /// In the forms of an event affordance.
    Event,
    /// In the top-level `forms` of the Thing.
    Thing,
}

/// A TD `ExpectedResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedResponse {
    /// Media type of the response.
    pub content_type: String,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A TD `AdditionalExpectedResponse`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalExpectedResponse {
    /// Whether this response means success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Media type of the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Name of a schema in the TD's `schemaDefinitions` that the body follows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A TD operation type (`op`), TD 1.1 §5.3.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    /// Read a property.
    ReadProperty,
    /// Write a property.
    WriteProperty,
    /// Observe a property.
    ObserveProperty,
    /// Stop observing a property.
    UnobserveProperty,
    /// Invoke an action.
    InvokeAction,
    /// Query the status of an action invocation.
    QueryAction,
    /// Cancel an action invocation.
    CancelAction,
    /// Subscribe to an event.
    SubscribeEvent,
    /// Unsubscribe from an event.
    UnsubscribeEvent,
    /// Read all properties at once.
    ReadAllProperties,
    /// Write all properties at once.
    WriteAllProperties,
    /// Read several properties at once.
    ReadMultipleProperties,
    /// Write several properties at once.
    WriteMultipleProperties,
    /// Observe all properties.
    ObserveAllProperties,
    /// Stop observing all properties.
    UnobserveAllProperties,
    /// Query the status of all action invocations.
    QueryAllActions,
    /// Subscribe to all events.
    SubscribeAllEvents,
    /// Unsubscribe from all events.
    UnsubscribeAllEvents,
}

impl Operation {
    /// Which kind of form may use this operation.
    pub fn scope(self) -> OperationScope {
        use Operation::*;
        match self {
            ReadProperty | WriteProperty | ObserveProperty | UnobserveProperty => {
                OperationScope::Property
            }
            InvokeAction | QueryAction | CancelAction => OperationScope::Action,
            SubscribeEvent | UnsubscribeEvent => OperationScope::Event,
            _ => OperationScope::Thing,
        }
    }
}

/// A TD `Form` (TD 1.1 §5.3.4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Form {
    /// Target URI. Relative URIs are resolved against the TD's `base`.
    pub href: String,
    /// The operations this form performs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<OneOrMany<Operation>>,
    /// Media type of the request and response bodies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Content coding, such as `gzip`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_coding: Option<String>,
    /// Sub-protocol, such as `sse` or `longpoll`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subprotocol: Option<String>,
    /// Security definitions that apply to this form, overriding the Thing's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<OneOrMany<String>>,
    /// OAuth2 scopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<OneOrMany<String>>,
    /// The expected response, if it differs from the form's `contentType`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ExpectedResponse>,
    /// Other responses, such as error responses or alternative content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_responses: Option<Vec<AdditionalExpectedResponse>>,
    /// Other terms, such as `htv:methodName`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Form {
    /// A form with only an `href`.
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            op: None,
            content_type: None,
            content_coding: None,
            subprotocol: None,
            security: None,
            scopes: None,
            response: None,
            additional_responses: None,
            extra: Map::new(),
        }
    }

    /// Sets the operations.
    pub fn with_op(mut self, op: impl Into<OneOrMany<Operation>>) -> Self {
        self.op = Some(op.into());
        self
    }

    /// Sets the content type.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Sets the sub-protocol.
    pub fn with_subprotocol(mut self, subprotocol: impl Into<String>) -> Self {
        self.subprotocol = Some(subprotocol.into());
        self
    }

    /// Adds an extension term, such as `htv:methodName`.
    pub fn with_term(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(name.into(), value.into());
        self
    }

    /// The operations of this form, if any are given.
    pub fn operations(&self) -> impl Iterator<Item = Operation> + '_ {
        self.op.iter().flat_map(|op| op.iter().copied())
    }
}
