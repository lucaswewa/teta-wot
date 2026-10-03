//! Thing types and their definitions.

use std::any::{Any, TypeId};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::action::ActionSpec;
use crate::device::{Device, DeviceAccess, DeviceControl, Driver};
use crate::endpoint::EndpointSpec;
use crate::event::EventSpec;
use crate::inprocess::ThingRef;
use crate::property::PropertySpec;
use crate::runtime::ThingHandle;
use crate::server::Server;
use crate::slots::{SlotAccess, SlotControl, SlotField, SlotSelection};
use crate::stream::MjpegStream;

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

/// What a Thing knows about its place on the server: its lifecycle hooks
/// receive one.
#[derive(Debug, Clone)]
pub struct ThingCtx {
    name: Arc<str>,
    server: Server,
}

impl ThingCtx {
    pub(crate) fn new(name: Arc<str>, server: Server) -> Self {
        Self { name, server }
    }

    /// The name the Thing is served under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The server: other Things, services, the application configuration.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The Thing's own folder for persistent files, if the
    /// server persists settings.
    pub fn settings_folder(&self) -> Option<PathBuf> {
        self.server
            .settings_folder()
            .map(|folder| folder.join(&*self.name))
    }
}

type DeviceBuild<T> = Box<dyn FnOnce(&Arc<T>) -> Arc<dyn DeviceControl> + Send>;
type SlotBuild<T> = Box<dyn FnOnce(&Arc<T>) -> Arc<dyn SlotControl> + Send>;
/// Makes a provided interface (an `Arc<dyn Trait>`, boxed as `Any`) for a Thing.
pub(crate) type InterfaceBuild =
    Arc<dyn Fn(&Arc<ThingHandle>) -> Option<Box<dyn Any + Send + Sync>> + Send + Sync>;
type StateFn<T> = Box<dyn Fn(&T) -> Value + Send + Sync>;
type StreamAccessor<T> = fn(&T) -> &MjpegStream;

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
    pub(crate) events: Vec<(String, EventSpec<T>)>,
    pub(crate) streams: Vec<(String, StreamAccessor<T>)>,
    pub(crate) devices: Vec<(String, DeviceBuild<T>)>,
    pub(crate) endpoints: Vec<EndpointSpec<T>>,
    pub(crate) class_name: Option<String>,
    pub(crate) slots: Vec<(String, SlotBuild<T>)>,
    pub(crate) interfaces: Vec<(TypeId, InterfaceBuild)>,
    pub(crate) services: Vec<(TypeId, &'static str)>,
    pub(crate) thing_state: Option<StateFn<T>>,
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
            events: Vec::new(),
            streams: Vec::new(),
            devices: Vec::new(),
            endpoints: Vec::new(),
            class_name: None,
            slots: Vec::new(),
            interfaces: Vec::new(),
            services: Vec::new(),
            thing_state: None,
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

    /// Sets the class name used in the settings file name,
    /// `Settings-{class_name}.json`.
    pub fn class_name(mut self, class_name: impl Into<String>) -> Self {
        self.class_name = Some(class_name.into());
        self
    }

    /// Adds a slot: a field ([`Slot`](crate::Slot),
    /// [`OptSlot`](crate::OptSlot) or [`SlotMap`](crate::SlotMap)) that the
    /// runtime connects to other Things, by default as `default` says, or as
    /// the configuration's `thing_slots` overrides.
    pub fn slot<F: SlotField>(
        mut self,
        name: impl Into<String>,
        accessor: fn(&T) -> &F,
        default: SlotSelection,
    ) -> Self {
        self.slots.push((
            name.into(),
            Box::new(move |thing| {
                Arc::new(SlotAccess {
                    thing: Arc::clone(thing),
                    accessor,
                    default,
                }) as Arc<dyn SlotControl>
            }),
        ));
        self
    }

    /// Declares that this Thing provides the interface `I` (a `dyn Trait`
    /// declared with `#[wot::interface]`), so that slots of that interface
    /// can connect to it. `cast` turns a [`ThingRef`] to the Thing into the
    /// interface: usually `|r| Arc::new(r)`, with the trait implemented for
    /// `ThingRef<Self>`.
    pub fn interface<I: ?Sized + Send + Sync + 'static>(
        mut self,
        cast: fn(ThingRef<T>) -> Arc<I>,
    ) -> Self {
        let build: InterfaceBuild = Arc::new(move |handle: &Arc<ThingHandle>| {
            let thing = ThingHandle::thing_ref::<T>(handle)?;
            Some(Box::new(cast(thing)) as Box<dyn Any + Send + Sync>)
        });
        self.interfaces.push((TypeId::of::<I>(), build));
        self
    }

    /// Declares that the Thing needs the service `S` (for a `Dep<S>`
    /// parameter): the runtime refuses to build without it.
    pub fn requires_service<S: ?Sized + Send + Sync + 'static>(mut self) -> Self {
        self.services
            .push((TypeId::of::<S>(), std::any::type_name::<S>()));
        self
    }

    /// Sets how the Thing summarises its state for
    /// [`Server::thing_states`].
    pub fn thing_state(mut self, state: impl Fn(&T) -> Value + Send + Sync + 'static) -> Self {
        self.thing_state = Some(Box::new(state));
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

    /// Adds an event: an [`Event`](crate::Event) field of the Thing.
    pub fn event(mut self, name: impl Into<String>, event: EventSpec<T>) -> Self {
        self.events.push((name.into(), event));
        self
    }

    /// Adds an MJPEG stream held in a field: served at `/{thing}/{name}`, with a viewer
    /// page at `/{thing}/{name}/viewer`, and linked from the TD.
    /// Its name shares the affordances' namespace.
    pub fn stream(mut self, name: impl Into<String>, accessor: fn(&T) -> &MjpegStream) -> Self {
        self.streams.push((name.into(), accessor));
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
