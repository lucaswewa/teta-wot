//! The Thing Description document.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::data_schema::{MultiLanguage, push_semantic_type};
use crate::form::OperationScope;
use crate::{
    ActionAffordance, Context, ContextEntry, DataSchema, EventAffordance, Form, Link, OneOrMany,
    PropertyAffordance, SecurityScheme, TdError,
};

/// The no security name.
pub const NO_SECURITY_NAME: &str = "no_security";

/// Version information (`version`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Version of this TD instance.
    pub instance: String,
    /// Version of the Thing Model it was built from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A W3C WoT Thing Description, TD 1.1 (§5.3.1.1).
///
/// Build one with [`ThingDescription::builder`], which checks the result, or
/// assemble the struct directly and call [`validate`](Self::validate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThingDescription {
    /// Identifier of the Thing (a URI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Human-readable title.
    pub title: String,
    /// Titles in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titles: Option<MultiLanguage>,
    /// Property affordances, by name.
    #[serde(default)]
    pub properties: IndexMap<String, PropertyAffordance>,
    /// Action affordances, by name.
    #[serde(default)]
    pub actions: IndexMap<String, ActionAffordance>,
    /// Event affordances, by name.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub events: IndexMap<String, EventAffordance>,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Descriptions in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<MultiLanguage>,
    /// Version information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionInfo>,
    /// Links to other resources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Forms for operations on the whole Thing, such as `readallproperties`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<Form>,
    /// Base URI that relative `href`s are resolved against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Security schemes, by name.
    pub security_definitions: IndexMap<String, SecurityScheme>,
    /// URI of support information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support: Option<String>,
    /// When the TD was created (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// When the TD was last modified (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// Names of the security definitions that apply to the whole Thing.
    pub security: OneOrMany<String>,
    /// Profiles the Thing conforms to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<OneOrMany<String>>,
    /// Named data schemas, referred to from `additionalResponses`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_definitions: Option<IndexMap<String, DataSchema>>,
    /// Schemas of URI template variables used in the top-level forms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri_variables: Option<IndexMap<String, DataSchema>>,
    /// Semantic annotations.
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<OneOrMany<String>>,
    /// The JSON-LD context.
    #[serde(rename = "@context")]
    pub context: Context,
    /// Terms that TD 1.1 doesn't define. Keys must not repeat a typed term.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ThingDescription {
    /// Starts building a TD with the given title.
    pub fn builder(title: impl Into<String>) -> ThingBuilder {
        ThingBuilder::new(title.into())
    }

    /// Checks the rules that the type system doesn't enforce:
    ///
    /// - the `@context` rules for TD 1.1 ([`Context::validate`]);
    /// - there is at least one security definition, and every security name
    ///   used (by the Thing, by forms, or by combo schemes) is defined;
    /// - every affordance has a non-empty name and at least one form;
    /// - every form uses only operations that belong where it is.
    ///
    /// Validation against the W3C JSON Schema is separate
    /// (`validation` feature).
    pub fn validate(&self) -> Result<(), TdError> {
        self.context.validate()?;
        if self.security_definitions.is_empty() {
            return Err(TdError::NoSecurityDefinitions);
        }
        let check_security = |names: &OneOrMany<String>| {
            names.iter().try_for_each(|name| {
                if self.security_definitions.contains_key(name) {
                    Ok(())
                } else {
                    Err(TdError::UndefinedSecurity(name.clone()))
                }
            })
        };
        check_security(&self.security)?;
        for scheme in self.security_definitions.values() {
            for name in scheme.referenced_schemes() {
                if !self.security_definitions.contains_key(name) {
                    return Err(TdError::UndefinedSecurity(name.to_owned()));
                }
            }
        }

        let affordances = self
            .properties
            .iter()
            .map(|(n, p)| (OperationScope::Property, n, &p.forms))
            .chain(
                self.actions
                    .iter()
                    .map(|(n, a)| (OperationScope::Action, n, &a.forms)),
            )
            .chain(
                self.events
                    .iter()
                    .map(|(n, e)| (OperationScope::Event, n, &e.forms)),
            );
        for (scope, name, forms) in affordances {
            if name.is_empty() {
                return Err(TdError::EmptyName { scope });
            }
            if forms.is_empty() {
                return Err(TdError::NoForms {
                    scope,
                    name: name.clone(),
                });
            }
            check_forms(scope, name, forms, &check_security)?;
        }
        check_forms(OperationScope::Thing, "", &self.forms, &check_security)
    }
}

fn check_forms(
    scope: OperationScope,
    name: &str,
    forms: &[Form],
    check_security: &impl Fn(&OneOrMany<String>) -> Result<(), TdError>,
) -> Result<(), TdError> {
    for form in forms {
        if let Some(op) = form.operations().find(|op| op.scope() != scope) {
            return Err(TdError::MisplacedOperation {
                scope,
                name: name.to_owned(),
                op,
            });
        }
        if let Some(security) = &form.security {
            check_security(security)?;
        }
    }
    Ok(())
}

/// Builds a [`ThingDescription`] and checks it with
/// [`ThingDescription::validate`].
///
/// If no security definition is added, the TD gets:
/// `"security": "no_security"` with a `nosec` scheme described as
/// "No security".
#[derive(Debug, Clone)]
#[must_use]
pub struct ThingBuilder {
    td: ThingDescription,
    security_set: bool,
    duplicate: Option<TdError>,
}

impl ThingBuilder {
    fn new(title: String) -> Self {
        Self {
            td: ThingDescription {
                id: None,
                title,
                titles: None,
                properties: IndexMap::new(),
                actions: IndexMap::new(),
                events: IndexMap::new(),
                description: None,
                descriptions: None,
                version: None,
                links: Vec::new(),
                forms: Vec::new(),
                base: None,
                security_definitions: IndexMap::new(),
                support: None,
                created: None,
                modified: None,
                security: OneOrMany::Many(Vec::new()),
                profile: None,
                schema_definitions: None,
                uri_variables: None,
                semantic_type: None,
                context: Context::default(),
                extra: Map::new(),
            },
            security_set: false,
            duplicate: None,
        }
    }

    /// Sets the identifier (a URI, such as `urn:uuid:…`).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.td.id = Some(id.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.td.description = Some(description.into());
        self
    }

    /// Sets the base URI.
    pub fn base(mut self, base: impl Into<String>) -> Self {
        self.td.base = Some(base.into());
        self
    }

    /// Sets the version of this TD instance.
    pub fn version(mut self, instance: impl Into<String>) -> Self {
        self.td.version = Some(VersionInfo {
            instance: instance.into(),
            model: None,
            extra: Map::new(),
        });
        self
    }

    /// Adds a semantic annotation (`@type`).
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        push_semantic_type(&mut self.td.semantic_type, semantic_type.into());
        self
    }

    /// Adds a context entry after the TD 1.1 URI.
    pub fn context_entry(mut self, entry: ContextEntry) -> Self {
        self.td.context.push(entry);
        self
    }

    /// Defines a prefix for semantic annotations, such as `saref`.
    pub fn context_prefix(mut self, prefix: impl Into<String>, uri: impl Into<String>) -> Self {
        self.td.context.add_prefix(prefix, uri);
        self
    }

    /// Adds a property. Names must be unique among the properties.
    pub fn property(
        mut self,
        name: impl Into<String>,
        property: impl Into<PropertyAffordance>,
    ) -> Self {
        let name = name.into();
        if self
            .td
            .properties
            .insert(name.clone(), property.into())
            .is_some()
        {
            self.note_duplicate(OperationScope::Property, name);
        }
        self
    }

    /// Adds an action. Names must be unique among the actions.
    pub fn action(mut self, name: impl Into<String>, action: impl Into<ActionAffordance>) -> Self {
        let name = name.into();
        if self
            .td
            .actions
            .insert(name.clone(), action.into())
            .is_some()
        {
            self.note_duplicate(OperationScope::Action, name);
        }
        self
    }

    /// Adds an event. Names must be unique among the events.
    pub fn event(mut self, name: impl Into<String>, event: impl Into<EventAffordance>) -> Self {
        let name = name.into();
        if self.td.events.insert(name.clone(), event.into()).is_some() {
            self.note_duplicate(OperationScope::Event, name);
        }
        self
    }

    fn note_duplicate(&mut self, scope: OperationScope, name: String) {
        self.duplicate
            .get_or_insert(TdError::DuplicateAffordance { scope, name });
    }

    /// Adds a link.
    pub fn link(mut self, link: Link) -> Self {
        self.td.links.push(link);
        self
    }

    /// Adds a top-level form.
    pub fn form(mut self, form: Form) -> Self {
        self.td.forms.push(form);
        self
    }

    /// Adds a security definition.
    pub fn security_definition(mut self, name: impl Into<String>, scheme: SecurityScheme) -> Self {
        self.td.security_definitions.insert(name.into(), scheme);
        self
    }

    /// Sets which security definitions apply to the whole Thing.
    pub fn security(mut self, names: impl Into<OneOrMany<String>>) -> Self {
        self.td.security = names.into();
        self.security_set = true;
        self
    }

    /// Adds an extension term at the top level.
    pub fn term(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.td.extra.insert(name.into(), value.into());
        self
    }

    /// Finishes the TD and checks it.
    pub fn build(mut self) -> Result<ThingDescription, TdError> {
        if let Some(duplicate) = self.duplicate {
            return Err(duplicate);
        }
        if self.td.security_definitions.is_empty() && !self.security_set {
            self.td.security_definitions.insert(
                NO_SECURITY_NAME.to_owned(),
                SecurityScheme::nosec().with_description("No security"),
            );
            self.td.security = OneOrMany::One(NO_SECURITY_NAME.to_owned());
        } else if !self.security_set {
            return Err(TdError::MissingSecurity);
        }
        self.td.validate()?;
        Ok(self.td)
    }
}
