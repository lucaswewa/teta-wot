//! Thing types and their definitions.

use std::future::Future;
use std::sync::Arc;

use crate::action::ActionSpec;
use crate::device::{Device, DeviceAccess, DeviceControl, Driver};
use crate::endpoint::EndpointSpec;
use crate::property::PropertySpec;

/// A Thing type.
///
/// One long-lived instance per configured name is shared as `Arc<Self>`,
/// and its affordances run concurrently. The definition says
/// which affordances it has; [`start`](Self::start) and
/// [`stop`](Self::stop) run when the server starts and stops.
///
/// `#[derive(Thing)]` implements it, or it can be implemented
/// by hand with [`ThingDefinition`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a Thing",
    label = "not a Thing",
    note = "add `#[derive(Thing)]` to the struct, or implement `Thing` with a `ThingDefinition`"
)]
pub trait Thing: Send + Sync + Sized + 'static {
    /// The Thing's title, description and affordances.
    fn definition() -> ThingDefinition<Self>;

    /// Runs once when the server starts, after the Thing's devices are
    /// open. An error stops the server from starting.
    fn start(self: Arc<Self>, ctx: ThingCtx) -> impl Future<Output = anyhow::Result<()>> + Send {
        let _ = ctx;
        async { Ok(()) }
    }

    /// Runs once when the server stops, before the Thing's devices close.
    fn stop(self: Arc<Self>, ctx: ThingCtx) -> impl Future<Output = ()> + Send {
        let _ = ctx;
        async {}
    }
}

/// What a Thing knows about its place on the server.
#[derive(Debug, Clone)]
pub struct ThingCtx {
    name: Arc<str>,
}

impl ThingCtx {
    pub(crate) fn new(name: Arc<str>) -> Self {
        Self { name }
    }

    /// The name the Thing is served under.
    pub fn name(&self) -> &str {
        &self.name
    }
}

type DeviceBuild<T> = Box<dyn FnOnce(&Arc<T>) -> Arc<dyn DeviceControl> + Send>;

/// The affordances and metadata of a Thing type, built with chained calls.
///
/// ```
/// use std::sync::Arc;
/// use teta_wot_core::{Action, ActionCtx, ActionError, DataProperty, NoInput, Prop, Thing, ThingDefinition};
///
/// struct Counter {
///     count: Prop<i64>,
/// }
///
/// impl Thing for Counter {
///     fn definition() -> ThingDefinition<Self> {
///         ThingDefinition::new("Counter")
///             .description("Counts.")
///             .property("count", DataProperty::new(|t: &Counter| &t.count).read_only().doc("The count."))
///             .action(
///                 "increment",
///                 Action::new(|t: Arc<Counter>, _ctx: ActionCtx, _: NoInput| async move {
///                     t.count.update(|c| *c += 1)?;
///                     Ok::<_, ActionError>(())
///                 })
///                 .doc("Add one."),
///             )
///     }
/// }
/// ```
pub struct ThingDefinition<T> {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) semantic_types: Vec<String>,
    pub(crate) context_prefixes: Vec<(String, String)>,
    pub(crate) properties: Vec<(String, PropertySpec<T>)>,
    pub(crate) actions: Vec<(String, ActionSpec<T>)>,
    pub(crate) devices: Vec<(String, DeviceBuild<T>)>,
    pub(crate) endpoints: Vec<EndpointSpec<T>>,
}

impl<T: Thing> ThingDefinition<T> {
    /// A definition with a title and no affordances.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            semantic_types: Vec::new(),
            context_prefixes: Vec::new(),
            properties: Vec::new(),
            actions: Vec::new(),
            devices: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a semantic annotation (`@type`) to the Thing.
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        self.semantic_types.push(semantic_type.into());
        self
    }

    /// Adds a prefix to the TD's `@context`, for semantic types and units
    /// written as compact IRIs (`saref:LightSwitch`).
    pub fn context_prefix(mut self, prefix: impl Into<String>, iri: impl Into<String>) -> Self {
        self.context_prefixes.push((prefix.into(), iri.into()));
        self
    }

    /// Adds a custom HTTP endpoint, made with
    /// `teta_wot::http::Endpoint`. Endpoints aren't in the Thing Description.
    pub fn endpoint(mut self, endpoint: impl Into<EndpointSpec<T>>) -> Self {
        self.endpoints.push(endpoint.into());
        self
    }

    /// Adds a property ([`DataProperty`](crate::DataProperty) or
    /// [`FunctionalProperty`](crate::FunctionalProperty)).
    pub fn property(
        mut self,
        name: impl Into<String>,
        property: impl Into<PropertySpec<T>>,
    ) -> Self {
        self.properties.push((name.into(), property.into()));
        self
    }

    /// Adds an action.
    pub fn action(mut self, name: impl Into<String>, action: impl Into<ActionSpec<T>>) -> Self {
        self.actions.push((name.into(), action.into()));
        self
    }

    /// Declares a device field, so that it is opened before
    /// [`Thing::start`] and closed after [`Thing::stop`]. Its thread is
    /// named `device:<thing>:<name>`.
    pub fn device<D: Driver>(
        mut self,
        name: impl Into<String>,
        accessor: fn(&T) -> &Device<D>,
    ) -> Self {
        self.devices.push((
            name.into(),
            Box::new(move |thing| {
                Arc::new(DeviceAccess {
                    thing: Arc::clone(thing),
                    accessor,
                }) as Arc<dyn DeviceControl>
            }),
        ));
        self
    }
}

/// A Thing definition that can't work.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{thing}.{affordance}`: {message}")]
pub struct DefinitionError {
    /// The Thing's name.
    pub thing: String,
    /// The affordance's name.
    pub affordance: String,
    /// What is wrong.
    pub message: String,
}

impl DefinitionError {
    pub(crate) fn new(thing: &str, affordance: &str, message: impl Into<String>) -> Self {
        Self {
            thing: thing.to_owned(),
            affordance: affordance.to_owned(),
            message: message.into(),
        }
    }
}

/// Splits a docstring into a title and a description: the title is the first line;
/// the description is the rest if there are more than two lines and the second
/// is blank, otherwise the whole docstring.
pub fn split_docstring(doc: &str) -> (String, String) {
    let title = doc.lines().next().unwrap_or_default().trim().to_owned();
    let lines: Vec<&str> = doc.split('\n').collect();
    let description = if lines.len() > 2 && lines[1].trim().is_empty() {
        lines[2..].join("\n")
    } else {
        doc.to_owned()
    };
    (title, description)
}
