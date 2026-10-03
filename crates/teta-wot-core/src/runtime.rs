//! The runtime: the registry of running Things, invocations and lifecycle.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::{Map, Value};
use teta_wot_td::{
    ActionAffordance, DataSchema, DataType, EventAffordance, ExpectedResponse, Form, Link,
    Operation, PropertyAffordance, SecurityScheme, TdError, ThingDescription,
};

use tracing::Level;
use uuid::Uuid;

use crate::BoxFuture;
use crate::action::{ActionError, ActionHandler, ActionMeta};
use crate::blob::Serialised;
use crate::broker::MessageBroker;
use crate::cancel::CancelToken;
use crate::config::FromConfig;
use crate::context::{ActionCtx, CatchUnwind, InvocationScope};
use crate::device::DeviceControl;
use crate::endpoint::{self, EndpointEntry};
use crate::event::EventEntry;
use crate::inprocess::ThingRef;
use crate::invocation::{Invocation, InvocationManager, InvocationStatus};
use crate::lock::{DEFAULT_LOCK_TIMEOUT, GlobalLock};
use crate::logs::LogBuffer;
use crate::property::PropertyEntry;
use crate::reserved::affordance_name_problem;
use crate::server::{Server, ServerShared, Service};
use crate::settings::SettingsStore;
use crate::slots::{SlotControl, SlotSelection};
use crate::stream::{MJPEG_MEDIA_TYPE, MjpegStream};
use crate::thing::{DefinitionError, InterfaceBuild, Thing, ThingCtx};
use crate::validate::{LocItem, ValidationError};

/// State shared by every Thing on a runtime.
pub(crate) struct Shared {
    pub(crate) lock: Option<Arc<GlobalLock>>,
    pub(crate) lock_log_level: Level,
    pub(crate) broker: Arc<MessageBroker>,
    pub(crate) invocations: Arc<InvocationManager>,
    pub(crate) server: Server,
}

/// A runtime can't be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    /// A Thing name has characters that are not allowed.
    #[error("invalid Thing name `{0}`: names may only contain letters, digits, `-` and `_`")]
    InvalidName(String),
    /// Two Things have the same name.
    #[error("two Things are named `{0}`")]
    DuplicateThing(String),
    /// Two affordances of one Thing have the same name (they would share a URL).
    #[error("Thing `{thing}` has two affordances named `{name}`")]
    DuplicateAffordance {
        /// The Thing.
        thing: String,
        /// The repeated name.
        name: String,
    },
    /// An affordance is badly defined, or badly named.
    #[error(transparent)]
    Definition(#[from] DefinitionError),
    /// A Thing's configuration (its `kwargs`) doesn't fit its `Config` type.
    #[error("the configuration of Thing `{thing}` is invalid: {error}")]
    InvalidConfig {
        /// The Thing.
        thing: String,
        /// What is wrong.
        error: String,
    },
    /// A slot can't be connected (`ThingSlotError`).
    #[error("{0}")]
    Slot(String),
    /// A Thing needs a service (a `Dep<T>` parameter) that isn't registered.
    #[error(
        "Thing `{thing}` needs the service `{service}`, which isn't registered with the server"
    )]
    MissingService {
        /// The Thing.
        thing: String,
        /// The service's type.
        service: String,
    },
    /// A Thing's settings folder can't be created.
    #[error("couldn't create the settings folder {path}: {error}")]
    SettingsFolder {
        /// The folder.
        path: String,
        /// Why.
        error: std::io::Error,
    },
}

/// A Thing failed to start.
#[derive(Debug, thiserror::Error)]
#[error("Thing `{thing}` failed to start: {error:#}")]
pub struct StartupError {
    /// The Thing that failed.
    pub thing: String,
    /// Why.
    pub error: anyhow::Error,
}

type ThingFactory = Box<dyn FnOnce(&Arc<Shared>) -> Result<ThingHandle, BuildError> + Send>;

/// Builds a [`Runtime`].
#[must_use]
pub struct RuntimeBuilder {
    global_lock: bool,
    lock_timeout: Duration,
    lock_log_level: Level,
    things: Vec<(String, ThingFactory)>,
    slot_overrides: HashMap<String, IndexMap<String, SlotSelection>>,
    services: HashMap<TypeId, Service>,
    application_config: Option<Value>,
    settings_folder: Option<PathBuf>,
}

impl fmt::Debug for RuntimeBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeBuilder")
            .field("global_lock", &self.global_lock)
            .field(
                "things",
                &self.things.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl RuntimeBuilder {
    /// Enables the global lock (off by default).
    pub fn global_lock(mut self, enabled: bool) -> Self {
        self.global_lock = enabled;
        self
    }

    /// How long to wait for the global lock (50 ms by default).
    pub fn global_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// The level of the "Global lock was busy" log line (INFO by default).
    pub fn global_lock_log_level(mut self, level: Level) -> Self {
        self.lock_log_level = level;
        self
    }

    /// Adds a Thing under `name`. Things start in the order they are added
    /// and stop in reverse.
    pub fn thing<T: Thing>(self, name: impl Into<String>, thing: T) -> Self {
        self.thing_arc(name, Arc::new(thing))
    }

    /// Adds a Thing that is already shared.
    pub fn thing_arc<T: Thing>(mut self, name: impl Into<String>, thing: Arc<T>) -> Self {
        let name = name.into();
        let thing_name = name.clone();
        self.things.push((
            name,
            Box::new(move |shared| ThingHandle::instantiate(&thing_name, thing, shared)),
        ));
        self
    }

    /// Adds a Thing built from its configuration: `kwargs` is
    /// deserialised into `T::Config` when the runtime is built.
    pub fn thing_from_config<T: FromConfig>(self, name: impl Into<String>, kwargs: Value) -> Self {
        let kwargs = match kwargs {
            Value::Object(kwargs) => kwargs,
            Value::Null => Map::new(),
            other => {
                let name = name.into();
                return self.failing(
                    name.clone(),
                    format!("kwargs must be an object, not {other}"),
                );
            }
        };
        self.thing_from_args::<T>(name, Vec::new(), kwargs)
    }

    /// Adds a Thing built from a configuration file entry's `args` and
    /// `kwargs`. The Thing's `Config` is deserialised from `kwargs` (an
    /// object), or from `args` (a list, field by field) when there are no
    /// `kwargs`; giving both is an error.
    pub fn thing_from_args<T: FromConfig>(
        mut self,
        name: impl Into<String>,
        args: Vec<Value>,
        kwargs: Map<String, Value>,
    ) -> Self {
        let name = name.into();
        let thing_name = name.clone();
        self.things.push((
            name,
            Box::new(move |shared| {
                let invalid = |error: String| BuildError::InvalidConfig {
                    thing: thing_name.clone(),
                    error,
                };
                let value = match (args.is_empty(), kwargs.is_empty()) {
                    (false, false) => {
                        return Err(invalid(
                            "use either args or kwargs, not both: a Rust Thing's configuration is one typed value".to_owned(),
                        ));
                    }
                    (false, true) => Value::Array(args),
                    (true, _) => Value::Object(kwargs),
                };
                let config = serde_json::from_value::<T::Config>(value)
                    .map_err(|error| invalid(error.to_string()))?;
                ThingHandle::instantiate(&thing_name, Arc::new(T::from_config(config)), shared)
            }),
        ));
        self
    }

    fn failing(mut self, name: String, error: String) -> Self {
        let thing = name.clone();
        self.things.push((
            name,
            Box::new(move |_| Err(BuildError::InvalidConfig { thing, error })),
        ));
        self
    }

    /// Overrides where a Thing's slots connect (the configuration's
    /// `thing_slots` for that Thing), by slot name.
    pub fn thing_slots(
        mut self,
        thing: impl Into<String>,
        slots: impl IntoIterator<Item = (String, SlotSelection)>,
    ) -> Self {
        self.slot_overrides
            .entry(thing.into())
            .or_default()
            .extend(slots);
        self
    }

    /// Registers a shared service, which actions receive as `Dep<T>`
    /// parameters. `T` may be a `dyn Trait`.
    pub fn service<T: ?Sized + Send + Sync + 'static>(mut self, service: Arc<T>) -> Self {
        self.services.insert(
            TypeId::of::<T>(),
            Service {
                value: Box::new(service),
                type_name: std::any::type_name::<T>(),
            },
        );
        self
    }

    /// Sets the application configuration (the configuration file's
    /// `application_config`), which Things read with
    /// [`Server::application_config`].
    pub fn application_config(mut self, config: Value) -> Self {
        self.application_config = Some(config);
        self
    }

    /// Persists settings in `folder`: each Thing gets `{folder}/{name}/`,
    /// and its settings file there. Without a folder, settings
    /// are ordinary properties.
    pub fn settings_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.settings_folder = Some(folder.into());
        self
    }

    /// Checks the names, builds each Thing's registry entry, connects the
    /// slots and returns the runtime.
    pub fn build(self) -> Result<Runtime, BuildError> {
        let server = Server::new(ServerShared {
            things: std::sync::OnceLock::new(),
            services: self.services,
            application_config: self.application_config,
            settings_folder: self.settings_folder,
        });
        let shared = Arc::new(Shared {
            lock: self.global_lock.then(|| GlobalLock::new(self.lock_timeout)),
            lock_log_level: self.lock_log_level,
            broker: Arc::new(MessageBroker::new()),
            invocations: Arc::new(InvocationManager::new()),
            server: server.clone(),
        });
        let mut things = IndexMap::new();
        for (name, factory) in self.things {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(BuildError::InvalidName(name));
            }
            if things.contains_key(&name) {
                return Err(BuildError::DuplicateThing(name));
            }
            let handle = factory(&shared)?;
            for (service, type_name) in &handle.services {
                if !server.shared.services.contains_key(service) {
                    return Err(BuildError::MissingService {
                        thing: name.clone(),
                        service: (*type_name).to_owned(),
                    });
                }
            }
            // a settings folder for every Thing.
            if let Some(folder) = server.settings_folder() {
                let folder = folder.join(&name);
                std::fs::create_dir_all(&folder).map_err(|error| BuildError::SettingsFolder {
                    path: folder.display().to_string(),
                    error,
                })?;
            }
            things.insert(name, Arc::new(handle));
        }

        let mut dependencies: IndexMap<String, Vec<String>> = IndexMap::new();
        for (name, thing) in &things {
            let overrides = self.slot_overrides.get(name);
            for configured in overrides.into_iter().flat_map(|o| o.keys()) {
                if !thing.slots.iter().any(|(slot, _)| slot == configured) {
                    tracing::warn!(
                        "`thing_slots` of `{name}` configures `{configured}`, which isn't one of its slots"
                    );
                }
            }
            for (slot_name, slot) in &thing.slots {
                let configured = overrides.and_then(|o| o.get(slot_name));
                let connected =
                    crate::slots::connect(name, slot_name, &**slot, &things, configured)
                        .map_err(BuildError::Slot)?;
                dependencies
                    .entry(name.clone())
                    .or_default()
                    .extend(connected);
            }
        }
        let names: Vec<String> = things.keys().cloned().collect();
        let start_order = crate::slots::start_order(&names, &dependencies).unwrap_or_else(|cycle| {
            tracing::warn!(
                "the slots of {cycle:?} depend on each other in a cycle: starting every Thing in configuration order"
            );
            names.clone()
        });

        let _ = server.shared.things.set(
            things
                .iter()
                .map(|(name, handle)| (name.clone(), Arc::downgrade(handle)))
                .collect(),
        );
        Ok(Runtime {
            shared,
            things,
            start_order,
            started: Mutex::new(Vec::new()),
        })
    }
}

/// The running Things of a server: their registry entries, invocations,
/// message broker and global lock, and their start and stop.
pub struct Runtime {
    shared: Arc<Shared>,
    things: IndexMap<String, Arc<ThingHandle>>,
    start_order: Vec<String>,
    started: Mutex<Vec<Arc<ThingHandle>>>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("things", &self.things.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Runtime {
    /// Starts building a runtime.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder {
            global_lock: false,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            lock_log_level: Level::INFO,
            things: Vec::new(),
            slot_overrides: HashMap::new(),
            services: HashMap::new(),
            application_config: None,
            settings_folder: None,
        }
    }

    /// A Thing by name.
    pub fn thing(&self, name: &str) -> Option<&Arc<ThingHandle>> {
        self.things.get(name)
    }

    /// A typed reference to a Thing, for in-process calls, if
    /// the Thing named `name` is a `T`.
    pub fn thing_ref<T: Thing>(&self, name: &str) -> Option<ThingRef<T>> {
        ThingHandle::thing_ref(self.things.get(name)?)
    }

    /// Every Thing, in the order they were added.
    pub fn things(&self) -> impl Iterator<Item = &Arc<ThingHandle>> {
        self.things.values()
    }

    /// The invocations.
    pub fn invocations(&self) -> &Arc<InvocationManager> {
        &self.shared.invocations
    }

    /// The message broker.
    pub fn broker(&self) -> &Arc<MessageBroker> {
        &self.shared.broker
    }

    /// The global lock, if enabled.
    pub fn global_lock(&self) -> Option<&Arc<GlobalLock>> {
        self.shared.lock.as_ref()
    }

    /// The order the Things start in: the Things a Thing's slots connect to
    /// first, otherwise configuration order. They stop in reverse.
    pub fn start_order(&self) -> &[String] {
        &self.start_order
    }

    /// The server handle that the Things see.
    pub fn server(&self) -> &Server {
        &self.shared.server
    }

    /// Loads every Thing's settings, then starts the Things in
    /// [`start_order`](Self::start_order): for each, its devices open and
    /// then [`Thing::start`] runs. If one fails, the Things already started
    /// stop in reverse order and the failure is returned.
    pub async fn start(&self) -> Result<(), StartupError> {
        for thing in self.things.values() {
            thing.load_settings().await;
        }
        for name in &self.start_order {
            let thing = &self.things[name];
            if let Err(error) = thing.start().await {
                self.stop().await;
                return Err(StartupError {
                    thing: thing.name().to_owned(),
                    error,
                });
            }
            lock(&self.started).push(Arc::clone(thing));
        }
        Ok(())
    }

    /// Stops the started Things in reverse order: for each, [`Thing::stop`]
    /// runs and then its devices close.
    pub async fn stop(&self) {
        let started = std::mem::take(&mut *lock(&self.started));
        for thing in started.into_iter().rev() {
            thing.stop().await;
        }
    }

    /// Closes the message broker (ending observation streams), cancels
    /// every unfinished invocation, waits up to `grace` for them to finish,
    /// then stops the Things.
    pub async fn shutdown(&self, grace: Duration) {
        self.shared.broker.close();
        self.shared.invocations.cancel_all();
        let _ = tokio::time::timeout(grace, self.shared.invocations.wait_idle()).await;
        self.stop().await;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts and stops one Thing, whatever its type.
trait Lifecycle: Send + Sync {
    fn start(&self, ctx: ThingCtx) -> BoxFuture<'static, anyhow::Result<()>>;
    fn stop(&self, ctx: ThingCtx) -> BoxFuture<'static, ()>;
}

struct TypedLifecycle<T>(Arc<T>);

impl<T: Thing> Lifecycle for TypedLifecycle<T> {
    fn start(&self, ctx: ThingCtx) -> BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(Arc::clone(&self.0).start(ctx))
    }

    fn stop(&self, ctx: ThingCtx) -> BoxFuture<'static, ()> {
        Box::pin(Arc::clone(&self.0).stop(ctx))
    }
}

/// Where a Thing is served, for building its Thing Description.
#[derive(Debug, Clone, PartialEq)]
pub struct TdOptions {
    /// The Thing's path, with the trailing slash: `/{api_prefix}/{name}/`.
    /// Form `href`s are this path followed by the affordance's name.
    pub path: String,
    /// The base URL, if known.
    pub base: Option<String>,
    /// The TD's `id`, if any.
    pub id: Option<String>,
    /// Whether to describe observation and events:
    /// `observable` on data properties, with server-sent events forms at
    /// their paths, and the Thing's `events`. TDs have none of
    /// these, and without them events can't be described (an event needs a
    /// form), so they are left out. The HTTP binding always turns this on.
    pub observation: bool,
    /// The Thing's WebSocket URL (`ws://host/{prefix}/{thing}/ws`), for
    /// WebSocket forms next to the SSE ones, if `observation` is on.
    pub websocket: Option<String>,
    /// Whether to list MJPEG streams, their viewer pages and linked custom
    /// endpoints in the TD's `links`; the
    /// HTTP binding always turns this on.
    pub links: bool,
    /// Whether to add the Thing's top-level forms:
    /// `readallproperties` and `writemultipleproperties` at
    /// `{path}properties`, with `observeallproperties` there over
    /// server-sent events, `queryallactions` at `{path}actions`, and
    /// `subscribeallevents` at `{path}events`. Each form is left out if the
    /// Thing has nothing it applies to.
    pub top_level: bool,
    /// Where invocations are served, such as `/action_invocations/`. If set,
    /// every action says whether it is `synchronous`, and asynchronous ones
    /// get a `queryaction` and `cancelaction` form at `{invocations}{id}`,
    /// with `id` among the action's `uriVariables`. An action that
    /// returns a Blob also gets a form for its download,
    /// `{invocations}{id}/output`, whose response has the Blob's media type.
    pub invocations: Option<String>,
    /// Whether synchronous actions are served synchronously (the HTTP
    /// binding's `wot` profile). If not, every action is asynchronous.
    pub synchronous_actions: bool,
    /// The WoT Profiles the TD conforms to (its `profile`).
    pub profiles: Vec<String>,
    /// The security scheme every form requires, with its name. `None` is
    /// `nosec` scheme.
    pub security: Option<(String, SecurityScheme)>,
}

impl TdOptions {
    /// Options for a Thing served at `/{name}/` with no API prefix: no base, no `id`, no observation.
    pub fn for_name(name: &str) -> Self {
        Self {
            path: format!("/{name}/"),
            base: None,
            id: None,
            observation: false,
            websocket: None,
            links: false,
            top_level: false,
            invocations: None,
            synchronous_actions: false,
            profiles: Vec::new(),
            security: None,
        }
    }
}

/// One running Thing: its instance and the registry entries of its
/// affordances, which the HTTP binding serves.
pub struct ThingHandle {
    name: Arc<str>,
    title: String,
    description: Option<String>,
    semantic_types: Vec<String>,
    context_prefixes: Vec<(String, String)>,
    properties: IndexMap<String, PropertyEntry>,
    actions: IndexMap<String, ActionEntry>,
    events: IndexMap<String, EventEntry>,
    streams: IndexMap<String, MjpegStream>,
    endpoints: Vec<EndpointEntry>,
    devices: Vec<(String, Arc<dyn DeviceControl>)>,
    class_name: String,
    slots: Vec<(String, Arc<dyn SlotControl>)>,
    interfaces: Vec<(TypeId, InterfaceBuild)>,
    services: Vec<(TypeId, &'static str)>,
    thing_state: Option<Box<dyn Fn() -> Value + Send + Sync>>,
    settings: Option<Arc<SettingsStore>>,
    server: Server,
    lifecycle: Arc<dyn Lifecycle>,
    instance: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for ThingHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThingHandle")
            .field("name", &self.name)
            .field("properties", &self.properties.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ThingHandle {
    fn instantiate<T: Thing>(
        name: &str,
        thing: Arc<T>,
        shared: &Arc<Shared>,
    ) -> Result<Self, BuildError> {
        let definition = T::definition();
        let thing_name: Arc<str> = name.into();
        let mut seen = std::collections::HashSet::new();
        let mut check = |affordance: &str| {
            if let Some(problem) = affordance_name_problem(affordance) {
                return Err(BuildError::Definition(DefinitionError::new(
                    name, affordance, problem,
                )));
            }
            if seen.insert(affordance.to_owned()) {
                Ok(())
            } else {
                Err(BuildError::DuplicateAffordance {
                    thing: name.to_owned(),
                    name: affordance.to_owned(),
                })
            }
        };

        let class_name = definition
            .class_name
            .clone()
            .unwrap_or_else(|| definition.title.clone());
        let settings = shared
            .server
            .settings_folder()
            .map(|folder| Arc::new(SettingsStore::new(folder, &thing_name, &class_name)));

        let mut properties = IndexMap::new();
        for (property_name, spec) in definition.properties {
            check(&property_name)?;
            let handler = (spec.build)(&thing, name, &property_name, shared, settings.as_ref())?;
            let meta = spec.meta;
            properties.insert(
                property_name.clone(),
                PropertyEntry {
                    title: meta.title.unwrap_or_else(|| property_name.clone()),
                    name: property_name,
                    thing: Arc::clone(&thing_name),
                    description: meta.description,
                    read_only: meta.read_only.unwrap_or(spec.default_read_only),
                    global_lock: meta.global_lock.unwrap_or(true),
                    unit: meta.unit,
                    semantic_types: meta.semantic_types,
                    handler,
                    shared: Arc::clone(shared),
                },
            );
        }

        let mut actions = IndexMap::new();
        for (action_name, spec) in definition.actions {
            check(&action_name)?;
            let handler = (spec.build)(&thing, name, &action_name)?;
            actions.insert(
                action_name.clone(),
                ActionEntry {
                    title: spec
                        .meta
                        .title
                        .clone()
                        .unwrap_or_else(|| action_name.clone()),
                    name: action_name.into(),
                    thing: Arc::clone(&thing_name),
                    meta: spec.meta,
                    handler,
                    shared: Arc::clone(shared),
                },
            );
        }

        let mut events = IndexMap::new();
        for (event_name, spec) in definition.events {
            check(&event_name)?;
            let data = (spec.build)(&thing, name, &event_name, &shared.broker)?;
            let meta = spec.meta;
            events.insert(
                event_name.clone(),
                EventEntry {
                    title: meta.title.unwrap_or_else(|| event_name.clone()),
                    name: event_name,
                    description: meta.description,
                    semantic_types: meta.semantic_types,
                    data,
                },
            );
        }

        let mut streams = IndexMap::new();
        for (stream_name, accessor) in definition.streams {
            check(&stream_name)?;
            streams.insert(stream_name, accessor(&thing).clone());
        }

        let mut endpoints: Vec<EndpointEntry> = Vec::new();
        for spec in definition.endpoints {
            if let Some(problem) = endpoint::path_problem(&spec.path) {
                return Err(DefinitionError::new(name, &spec.path, problem).into());
            }
            if endpoints
                .iter()
                .any(|e| e.method == spec.method && e.path == spec.path)
            {
                return Err(DefinitionError::new(
                    name,
                    &spec.path,
                    format!("two endpoints answer {} {}", spec.method, spec.path),
                )
                .into());
            }
            endpoints.push(EndpointEntry {
                handler: (spec.build)(&thing),
                method: spec.method,
                path: spec.path,
                description: spec.description,
                link: spec.link,
                name: spec.name,
            });
        }

        let devices = definition
            .devices
            .into_iter()
            .map(|(device_name, build)| (device_name, build(&thing)))
            .collect();
        let slots = definition
            .slots
            .into_iter()
            .map(|(slot_name, build)| (slot_name, build(&thing)))
            .collect();
        let thing_state = definition.thing_state.map(|state| {
            let thing = Arc::clone(&thing);
            Box::new(move || state(&thing)) as Box<dyn Fn() -> Value + Send + Sync>
        });

        Ok(Self {
            name: thing_name,
            title: definition.title,
            description: definition.description,
            semantic_types: definition.semantic_types,
            context_prefixes: definition.context_prefixes,
            properties,
            actions,
            events,
            streams,
            endpoints,
            devices,
            class_name,
            slots,
            interfaces: definition.interfaces,
            services: definition.services,
            thing_state,
            settings: settings.filter(|s| s.has_settings()),
            server: shared.server.clone(),
            lifecycle: Arc::new(TypedLifecycle(Arc::clone(&thing))),
            instance: thing,
        })
    }

    /// The class name used for the settings file (`Settings-{class}.json`).
    pub fn class_name(&self) -> &str {
        &self.class_name
    }

    /// The Thing's settings file, if it has settings and the server has a
    /// settings folder.
    pub fn settings_file(&self) -> Option<&std::path::Path> {
        self.settings.as_ref().map(|s| s.path())
    }

    /// Loads the Thing's settings file, if it has one.
    pub(crate) async fn load_settings(&self) {
        if let Some(settings) = &self.settings {
            settings.load().await;
        }
    }

    /// The Thing's state, for [`Server::thing_states`]: its
    /// `thing_state` function's value, or `{}`.
    pub fn thing_state(&self) -> Value {
        self.thing_state
            .as_ref()
            .map_or_else(|| Value::Object(Map::new()), |state| state())
    }

    /// The Thing as the interface `I`, if it declared that it provides it.
    pub fn interface<I: ?Sized + 'static>(this: &Arc<Self>) -> Option<Arc<I>> {
        let (_, build) = this
            .interfaces
            .iter()
            .find(|(key, _)| *key == TypeId::of::<I>())?;
        build(this)?.downcast::<Arc<I>>().ok().map(|boxed| *boxed)
    }

    /// The names of the Thing's slots.
    pub fn slot_names(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().map(|(name, _)| name.as_str())
    }

    /// The name the Thing is served under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The Thing's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The Thing's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The Thing itself, if it is a `T`.
    pub fn instance<T: Thing>(&self) -> Option<Arc<T>> {
        Arc::clone(&self.instance).downcast::<T>().ok()
    }

    /// A typed reference to the Thing, for in-process calls, if
    /// it is a `T`.
    pub fn thing_ref<T: Thing>(this: &Arc<Self>) -> Option<ThingRef<T>> {
        Some(ThingRef::new(Arc::clone(this), this.instance()?))
    }

    /// The Thing's semantic annotations (`@type`).
    pub fn semantic_types(&self) -> &[String] {
        &self.semantic_types
    }

    /// The custom endpoints, in definition order.
    pub fn endpoints(&self) -> impl Iterator<Item = &EndpointEntry> {
        self.endpoints.iter()
    }

    /// A property by name.
    pub fn property(&self, name: &str) -> Option<&PropertyEntry> {
        self.properties.get(name)
    }

    /// The properties, in definition order.
    pub fn properties(&self) -> impl Iterator<Item = &PropertyEntry> {
        self.properties.values()
    }

    /// An action by name.
    pub fn action(&self, name: &str) -> Option<&ActionEntry> {
        self.actions.get(name)
    }

    /// The actions, in definition order.
    pub fn actions(&self) -> impl Iterator<Item = &ActionEntry> {
        self.actions.values()
    }

    /// The MJPEG streams, by name, in definition order.
    pub fn streams(&self) -> impl Iterator<Item = (&str, &MjpegStream)> {
        self.streams
            .iter()
            .map(|(name, stream)| (name.as_str(), stream))
    }

    /// An MJPEG stream by name.
    pub fn stream(&self, name: &str) -> Option<&MjpegStream> {
        self.streams.get(name)
    }

    /// An event by name.
    pub fn event(&self, name: &str) -> Option<&EventEntry> {
        self.events.get(name)
    }

    /// The events, in definition order.
    pub fn events(&self) -> impl Iterator<Item = &EventEntry> {
        self.events.values()
    }

    async fn start(&self) -> anyhow::Result<()> {
        let mut opened: Vec<&Arc<dyn DeviceControl>> = Vec::new();
        for (device_name, device) in &self.devices {
            if let Err(error) = device.open(&format!("{}:{device_name}", self.name)).await {
                close_all(opened).await;
                return Err(error.into());
            }
            opened.push(device);
        }
        if let Err(error) = self
            .lifecycle
            .start(ThingCtx::new(Arc::clone(&self.name), self.server.clone()))
            .await
        {
            close_all(opened).await;
            return Err(error);
        }
        Ok(())
    }

    async fn stop(&self) {
        self.lifecycle
            .stop(ThingCtx::new(Arc::clone(&self.name), self.server.clone()))
            .await;
        close_all(self.devices.iter().map(|(_, d)| d).collect()).await;
    }

    /// The Thing Description: properties and actions
    /// in alphabetical order, forms at `{path}{name}`, `readOnly` and
    /// `writeOnly` always written, action schemas titled `<name>_input` and
    /// `<name>_output`, and the `no_security` scheme.
    ///
    /// With [`TdOptions::observation`]: data properties are `observable`, with an SSE form (and a
    /// WebSocket form, given the URL), and events are described, in
    /// alphabetical order, with the same two forms. The other options add
    /// top-level forms, invocation forms, `profile` and
    /// security.
    pub fn thing_description(&self, options: &TdOptions) -> Result<ThingDescription, TdError> {
        let mut td = ThingDescription::builder(&self.title);
        if let Some(id) = &options.id {
            td = td.id(id);
        }
        if let Some(description) = &self.description {
            td = td.description(description);
        }
        if let Some(base) = &options.base {
            td = td.base(base);
        }
        if !options.profiles.is_empty() {
            td = td.profile(options.profiles.clone());
        }
        if let Some((name, scheme)) = &options.security {
            td = td
                .security_definition(name, scheme.clone())
                .security(name.clone());
        }
        for (prefix, iri) in &self.context_prefixes {
            td = td.context_prefix(prefix, iri);
        }
        for semantic_type in &self.semantic_types {
            td = td.semantic_type(semantic_type);
        }

        let mut properties: Vec<_> = self.properties.values().collect();
        properties.sort_by(|a, b| a.name.cmp(&b.name));
        for property in properties {
            let op = if property.read_only {
                vec![Operation::ReadProperty]
            } else {
                vec![Operation::ReadProperty, Operation::WriteProperty]
            };
            let href = format!("{}{}", options.path, property.name);
            let mut affordance = PropertyAffordance::builder(property.data_schema().clone())
                .title(&property.title)
                .read_only(property.read_only)
                .form(Form::new(&href).with_op(op));
            if options.observation && property.is_observable() {
                affordance = affordance.observable(true).form(
                    Form::new(&href)
                        .with_op([Operation::ObserveProperty, Operation::UnobserveProperty])
                        .with_subprotocol("sse"),
                );
                if let Some(websocket) = &options.websocket {
                    affordance =
                        affordance.form(Form::new(websocket).with_op([Operation::ObserveProperty]));
                }
            }
            if let Some(description) = &property.description {
                affordance = affordance.description(description);
            }
            if let Some(default) = property.default_value() {
                affordance = affordance.default_value(default);
            }
            if let Some(unit) = &property.unit {
                affordance = affordance.unit(unit);
            }
            for semantic_type in &property.semantic_types {
                affordance = affordance.semantic_type(semantic_type);
            }
            td = td.property(&property.name, affordance);
        }

        let mut actions: Vec<_> = self.actions.values().collect();
        actions.sort_by(|a, b| a.name.cmp(&b.name));
        for action in actions {
            let mut affordance = ActionAffordance::builder()
                .title(&action.title)
                .input(
                    action
                        .input_schema()
                        .clone()
                        .with_title(format!("{}_input", action.name)),
                )
                .output(
                    action
                        .output_schema()
                        .clone()
                        .with_title(format!("{}_output", action.name)),
                )
                .form(
                    Form::new(format!("{}{}", options.path, action.name))
                        .with_op([Operation::InvokeAction]),
                );
            if let Some(invocations) = &options.invocations {
                let synchronous = options.synchronous_actions && action.meta.synchronous;
                affordance = affordance.synchronous(synchronous);
                if !synchronous {
                    affordance = affordance
                        .uri_variable(
                            "id",
                            DataSchema::of(DataType::String)
                                .with_title("Invocation ID")
                                .with_format("uuid"),
                        )
                        .form(
                            Form::new(format!("{invocations}{{id}}"))
                                .with_op([Operation::QueryAction, Operation::CancelAction]),
                        );
                    if let Some(media_type) = crate::blob::schema_media_type(action.output_schema())
                    {
                        let mut form = Form::new(format!("{invocations}{{id}}/output"))
                            .with_op([Operation::QueryAction]);
                        form.response = Some(ExpectedResponse {
                            content_type: media_type.to_owned(),
                            extra: Map::new(),
                        });
                        affordance = affordance.form(form);
                    }
                }
            }
            if let Some(description) = &action.meta.description {
                affordance = affordance.description(description);
            }
            for semantic_type in &action.meta.semantic_types {
                affordance = affordance.semantic_type(semantic_type);
            }
            td = td.action(&*action.name, affordance);
        }

        let mut events: Vec<_> = self.events.values().collect();
        events.sort_by(|a, b| a.name.cmp(&b.name));
        for event in events.into_iter().filter(|_| options.observation) {
            let mut affordance = EventAffordance::builder().title(&event.title).form(
                Form::new(format!("{}{}", options.path, event.name))
                    .with_op([Operation::SubscribeEvent, Operation::UnsubscribeEvent])
                    .with_subprotocol("sse"),
            );
            if let Some(websocket) = &options.websocket {
                affordance =
                    affordance.form(Form::new(websocket).with_op([Operation::SubscribeEvent]));
            }
            if let Some(description) = &event.description {
                affordance = affordance.description(description);
            }
            if let Some(data) = &event.data {
                affordance = affordance.data(data.clone());
            }
            for semantic_type in &event.semantic_types {
                affordance = affordance.semantic_type(semantic_type);
            }
            td = td.event(&event.name, affordance);
        }

        if options.links {
            for name in self.streams.keys() {
                let href = format!("{}{name}", options.path);
                td = td
                    .link(
                        Link::new(&href)
                            .with_rel("alternate")
                            .with_media_type(MJPEG_MEDIA_TYPE),
                    )
                    .link(
                        Link::new(format!("{href}/viewer"))
                            .with_rel("alternate")
                            .with_media_type("text/html"),
                    );
            }
            for endpoint in &self.endpoints {
                if let Some(link) = &endpoint.link {
                    let mut entry =
                        Link::new(format!("{}{}", options.path, endpoint.path)).with_rel(&link.rel);
                    if let Some(media_type) = &link.media_type {
                        entry = entry.with_media_type(media_type);
                    }
                    td = td.link(entry);
                }
            }
        }
        if options.top_level {
            td = self.top_level_forms(td, options);
        }
        td.build()
    }

    /// The top-level forms, for what the Thing has.
    fn top_level_forms(
        &self,
        mut td: teta_wot_td::ThingBuilder,
        options: &TdOptions,
    ) -> teta_wot_td::ThingBuilder {
        let properties = format!("{}properties", options.path);
        let mut ops = Vec::new();
        if !self.properties.is_empty() {
            ops.push(Operation::ReadAllProperties);
        }
        if self.properties.values().any(|p| !p.read_only) {
            ops.push(Operation::WriteMultipleProperties);
        }
        if !ops.is_empty() {
            td = td.form(Form::new(&properties).with_op(ops));
        }
        if options.observation && self.properties.values().any(|p| p.is_observable()) {
            td = td.form(
                Form::new(&properties)
                    .with_op([
                        Operation::ObserveAllProperties,
                        Operation::UnobserveAllProperties,
                    ])
                    .with_subprotocol("sse"),
            );
        }
        if !self.actions.is_empty() {
            td = td.form(
                Form::new(format!("{}actions", options.path)).with_op([Operation::QueryAllActions]),
            );
        }
        if options.observation && !self.events.is_empty() {
            td = td.form(
                Form::new(format!("{}events", options.path))
                    .with_op([
                        Operation::SubscribeAllEvents,
                        Operation::UnsubscribeAllEvents,
                    ])
                    .with_subprotocol("sse"),
            );
        }
        td
    }
}

async fn close_all(devices: Vec<&Arc<dyn DeviceControl>>) {
    for device in devices.into_iter().rev() {
        if let Err(error) = device.close().await {
            tracing::warn!("closing a device failed: {error}");
        }
    }
}

/// One action of a running Thing: what clients (and the HTTP binding) use.
#[derive(Clone)]
pub struct ActionEntry {
    name: Arc<str>,
    thing: Arc<str>,
    title: String,
    meta: ActionMeta,
    handler: Arc<dyn ActionHandler>,
    shared: Arc<Shared>,
}

impl fmt::Debug for ActionEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionEntry")
            .field("thing", &self.thing)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl ActionEntry {
    /// The action's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The action's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The action's description.
    pub fn description(&self) -> Option<&str> {
        self.meta.description.as_deref()
    }

    /// How long finished invocations are kept.
    pub fn retention(&self) -> Duration {
        self.meta.retention
    }

    /// The input's DataSchema.
    pub fn input_schema(&self) -> &DataSchema {
        self.handler.input_schema()
    }

    /// The output's DataSchema.
    pub fn output_schema(&self) -> &DataSchema {
        self.handler.output_schema()
    }

    /// Whether the action has an output: its output isn't always `null`
    /// (such as an action returning `()`).
    pub fn has_output(&self) -> bool {
        !is_null_schema(self.handler.output_schema())
    }

    /// Whether the action is synchronous (see `Action::synchronous`).
    pub fn is_synchronous(&self) -> bool {
        self.meta.synchronous
    }

    /// Starts an invocation with a client's input and returns it at once,
    /// `pending`. The input is validated first (errors located under
    /// `body`); a missing body should be passed as `null`. Must be called
    /// within a tokio runtime.
    ///
    /// Expired invocations are removed first.
    pub fn invoke(&self, input: Value) -> Result<Arc<Invocation>, ValidationError> {
        let shared = &self.shared;
        shared.invocations.expire();
        // treats a `null` body as missing, which only inputs that
        // may be `null` (such as `NoInput`) accept.
        if input.is_null() && !accepts_null(self.handler.input_schema()) {
            return Err(ValidationError::missing(
                vec![LocItem::from("body")],
                Value::Null,
            ));
        }

        let id = Uuid::new_v4();
        // In-process calls from another invocation share its lock ownership.
        let lock_owner = InvocationScope::current().map_or(id, |s| s.lock_owner());
        let cancel = CancelToken::new();
        let logs = LogBuffer::register(id);
        let scope = InvocationScope::new(id, lock_owner, cancel.clone(), Arc::clone(&logs));
        let ctx = ActionCtx::new(
            scope.clone(),
            Arc::clone(&self.thing),
            shared.lock.clone(),
            shared.server.clone(),
        );
        let (echo, future) = self.handler.prepare(&input, ctx)?;

        let invocation = Arc::new(Invocation::new(
            id,
            Arc::clone(&self.thing),
            Arc::clone(&self.name),
            echo,
            self.meta.retention,
            cancel,
            logs,
            Arc::clone(&shared.broker),
        ));
        shared.invocations.insert(Arc::clone(&invocation));
        invocation.announce();

        let lock = match (&shared.lock, self.meta.global_lock) {
            (Some(lock), true) => Some((Arc::clone(lock), lock_owner)),
            _ => None,
        };
        tokio::spawn(scope.run(run_invocation(
            Arc::clone(&invocation),
            future,
            lock,
            shared.lock_log_level,
        )));
        Ok(invocation)
    }

    /// Calls the action in-process, as another Thing or a test
    /// would, and waits for its output:
    ///
    /// - the input is validated as a client's would be;
    /// - the global lock is held, if the action uses it, on behalf of the
    ///   calling invocation (so a caller that holds it doesn't block itself);
    /// - the action runs in the caller's invocation context (its
    ///   cancellation and its log), or in a new stand-alone one;
    /// - no invocation is recorded.
    ///
    /// Invalid input, a busy lock and the action's own failure are all
    /// returned as [`ActionError`]s.
    ///
    /// Blobs in the output are freed when the value is returned, unless
    /// something else holds them: deserialise them through
    /// [`ThingRef::call_action`], or call the typed `{Thing}Actions` method.
    pub async fn call(&self, input: Value) -> Result<Value, ActionError> {
        Ok(self.call_serialised(input).await?.value)
    }

    /// [`call`](Self::call), with the output's Blobs kept alive.
    pub(crate) async fn call_serialised(&self, input: Value) -> Result<Serialised, ActionError> {
        if input.is_null() && !accepts_null(self.handler.input_schema()) {
            return Err(ValidationError::missing(vec![LocItem::from("body")], Value::Null).into());
        }
        let handler = Arc::clone(&self.handler);
        self.run_in_process(move |ctx| async move {
            let (_, future) = handler.prepare(&input, ctx)?;
            future.await
        })
        .await
    }

    /// Validates an input as a client's would be, and returns it coerced.
    pub fn validate_input(&self, input: &Value) -> Result<Value, ValidationError> {
        self.handler.validate(input)
    }

    /// Runs `f` as an in-process call of this action (see [`call`](Self::call)):
    /// with the global lock and the caller's invocation context. The macros'
    /// typed wrappers use it after validating the input.
    #[doc(hidden)]
    pub async fn run_in_process<F, Fut, O>(&self, f: F) -> Result<O, ActionError>
    where
        F: FnOnce(ActionCtx) -> Fut,
        Fut: Future<Output = Result<O, ActionError>>,
    {
        let current = InvocationScope::current();
        let scope = current.clone().unwrap_or_else(InvocationScope::fake);
        let ctx = ActionCtx::new(
            scope.clone(),
            Arc::clone(&self.thing),
            self.shared.lock.clone(),
            self.shared.server.clone(),
        );
        let _guard = match (&self.shared.lock, self.meta.global_lock) {
            (Some(lock), true) => Some(lock.acquire(scope.lock_owner()).await?),
            _ => None,
        };
        let future = f(ctx);
        if current.is_some() {
            future.await
        } else {
            scope.run(future).await
        }
    }
}

/// Whether a schema allows `null`.
fn accepts_null(schema: &DataSchema) -> bool {
    match (&schema.data_type, &schema.one_of) {
        (Some(data_type), _) => *data_type == teta_wot_td::DataType::Null,
        (None, Some(branches)) => branches.iter().any(accepts_null),
        (None, None) => schema.enumeration.is_none() && schema.constant.is_none(),
    }
}

/// Whether a schema admits only `null`.
fn is_null_schema(schema: &DataSchema) -> bool {
    schema.data_type == Some(teta_wot_td::DataType::Null) && schema.one_of.is_none()
}

/// Runs an invocation to the end, inside its scope and span.
async fn run_invocation(
    invocation: Arc<Invocation>,
    future: BoxFuture<'static, Result<Serialised, ActionError>>,
    lock: Option<(Arc<GlobalLock>, Uuid)>,
    lock_log_level: Level,
) {
    let guard = match &lock {
        Some((lock, owner)) => match lock.acquire(*owner).await {
            Ok(guard) => Some(guard),
            Err(busy) => {
                log_at(
                    lock_log_level,
                    &format!("Global lock was busy: didn't run {}.", invocation.action()),
                );
                let error = ActionError::from(busy);
                invocation.finish(InvocationStatus::Error, None, Some(error.problem()));
                return;
            }
        },
        None => None,
    };

    invocation.set_running();
    let result = CatchUnwind(future)
        .await
        .unwrap_or_else(|payload| Err(ActionError::from_panic(&*payload)));
    drop(guard);

    match result {
        Ok(output) => invocation.finish(InvocationStatus::Completed, Some(output), None),
        Err(error) => {
            let status = if error.is_cancelled() {
                tracing::info!(target: "wot::things", "Invocation {} was cancelled.", invocation.id());
                InvocationStatus::Cancelled
            } else if error.is_handled() {
                tracing::error!(target: "wot::things", "{error}");
                InvocationStatus::Error
            } else {
                tracing::error!(
                    target: "wot::things",
                    exception_type = %error.title(),
                    traceback = %error.traceback(),
                    "{error}",
                );
                InvocationStatus::Error
            };
            invocation.finish(status, None, Some(error.problem()));
        }
    }
}

fn log_at(level: Level, message: &str) {
    match level {
        Level::ERROR => tracing::error!(target: "wot::things", "{message}"),
        Level::WARN => tracing::warn!(target: "wot::things", "{message}"),
        Level::INFO => tracing::info!(target: "wot::things", "{message}"),
        Level::DEBUG => tracing::debug!(target: "wot::things", "{message}"),
        Level::TRACE => tracing::trace!(target: "wot::things", "{message}"),
    }
}
