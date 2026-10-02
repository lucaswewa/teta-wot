//! The runtime: the registry of running Things, invocations and lifecycle.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::Value;
use teta_wot_td::{
    ActionAffordance, DataSchema, Form, Operation, PropertyAffordance, TdError, ThingDescription,
};
use tracing::Level;
use uuid::Uuid;

use crate::BoxFuture;
use crate::action::{ActionError, ActionHandler, ActionMeta};
use crate::broker::MessageBroker;
use crate::cancel::CancelToken;
use crate::config::FromConfig;
use crate::context::{ActionCtx, CatchUnwind, InvocationScope};
use crate::device::DeviceControl;
use crate::endpoint::{self, EndpointEntry};
use crate::inprocess::ThingRef;
use crate::invocation::{Invocation, InvocationManager, InvocationStatus};
use crate::lock::{DEFAULT_LOCK_TIMEOUT, GlobalLock};
use crate::logs::LogBuffer;
use crate::property::PropertyEntry;
use crate::reserved::affordance_name_problem;
use crate::thing::{DefinitionError, Thing, ThingCtx};
use crate::validate::{LocItem, ValidationError};

/// State shared by every Thing on a runtime.
pub(crate) struct Shared {
    pub(crate) lock: Option<Arc<GlobalLock>>,
    pub(crate) lock_log_level: Level,
    pub(crate) broker: Arc<MessageBroker>,
    pub(crate) invocations: Arc<InvocationManager>,
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
        error: serde_json::Error,
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
    pub fn thing_from_config<T: FromConfig>(
        mut self,
        name: impl Into<String>,
        kwargs: Value,
    ) -> Self {
        let name = name.into();
        let thing_name = name.clone();
        self.things.push((
            name,
            Box::new(move |shared| {
                let config = serde_json::from_value::<T::Config>(kwargs).map_err(|error| {
                    BuildError::InvalidConfig {
                        thing: thing_name.clone(),
                        error,
                    }
                })?;
                ThingHandle::instantiate(&thing_name, Arc::new(T::from_config(config)), shared)
            }),
        ));
        self
    }

    /// Checks the names, builds each Thing's registry entry and returns the runtime.
    pub fn build(self) -> Result<Runtime, BuildError> {
        let shared = Arc::new(Shared {
            lock: self.global_lock.then(|| GlobalLock::new(self.lock_timeout)),
            lock_log_level: self.lock_log_level,
            broker: Arc::new(MessageBroker::new()),
            invocations: Arc::new(InvocationManager::new()),
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
            things.insert(name, Arc::new(handle));
        }
        Ok(Runtime {
            shared,
            things,
            started: Mutex::new(Vec::new()),
        })
    }
}

/// The running Things of a server: their registry entries, invocations,
/// message broker and global lock, and their start and stop.
pub struct Runtime {
    shared: Arc<Shared>,
    things: IndexMap<String, Arc<ThingHandle>>,
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

    /// Starts the Things in order: for each, its devices open and then
    /// [`Thing::start`] runs. If one fails, the Things already started stop
    /// in reverse order and the failure is returned.
    pub async fn start(&self) -> Result<(), StartupError> {
        for thing in self.things.values() {
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

    /// Cancels every unfinished invocation, waits up to `grace` for them to
    /// finish, then stops the Things.
    pub async fn shutdown(&self, grace: Duration) {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdOptions {
    /// The Thing's path, with the trailing slash: `/{api_prefix}/{name}/`.
    /// Form `href`s are this path followed by the affordance's name.
    pub path: String,
    /// The base URL, if known.
    pub base: Option<String>,
    /// The TD's `id`, if any.
    pub id: Option<String>,
}

impl TdOptions {
    /// Options for a Thing served at `/{name}/` with no API prefix.
    pub fn for_name(name: &str) -> Self {
        Self {
            path: format!("/{name}/"),
            base: None,
            id: None,
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
    endpoints: Vec<EndpointEntry>,
    devices: Vec<(String, Arc<dyn DeviceControl>)>,
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

        let mut properties = IndexMap::new();
        for (property_name, spec) in definition.properties {
            check(&property_name)?;
            let handler = (spec.build)(&thing, name, &property_name, shared)?;
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
            });
        }

        let devices = definition
            .devices
            .into_iter()
            .map(|(device_name, build)| (device_name, build(&thing)))
            .collect();

        Ok(Self {
            name: thing_name,
            title: definition.title,
            description: definition.description,
            semantic_types: definition.semantic_types,
            context_prefixes: definition.context_prefixes,
            properties,
            actions,
            endpoints,
            devices,
            lifecycle: Arc::new(TypedLifecycle(Arc::clone(&thing))),
            instance: thing,
        })
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
            .start(ThingCtx::new(Arc::clone(&self.name)))
            .await
        {
            close_all(opened).await;
            return Err(error);
        }
        Ok(())
    }

    async fn stop(&self) {
        self.lifecycle
            .stop(ThingCtx::new(Arc::clone(&self.name)))
            .await;
        close_all(self.devices.iter().map(|(_, d)| d).collect()).await;
    }

    /// The Thing Description: properties and actions in alphabetical order, forms
    /// at `{path}{name}`, `readOnly` and `writeOnly` always written, action schemas
    /// titled `<name>_input` and `<name>_output`, and the `no_security` scheme.
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
            let mut affordance = PropertyAffordance::builder(property.data_schema().clone())
                .title(&property.title)
                .read_only(property.read_only)
                .form(Form::new(format!("{}{}", options.path, property.name)).with_op(op));
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
            if let Some(description) = &action.meta.description {
                affordance = affordance.description(description);
            }
            for semantic_type in &action.meta.semantic_types {
                affordance = affordance.semantic_type(semantic_type);
            }
            td = td.action(&*action.name, affordance);
        }
        td.build()
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

    /// Starts an invocation with a client's input and returns it at once,
    /// `pending`. The input is validated first (errors located under
    /// `body`); a missing body should be passed as `null`. Must be called
    /// within a tokio runtime.
    ///
    /// Expired invocations are removed first.
    pub fn invoke(&self, input: Value) -> Result<Arc<Invocation>, ValidationError> {
        let shared = &self.shared;
        shared.invocations.expire();
        // Treats a `null` body as missing, which only inputs that
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
        let ctx = ActionCtx::new(scope.clone(), Arc::clone(&self.thing), shared.lock.clone());
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
    pub async fn call(&self, input: Value) -> Result<Value, ActionError> {
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

/// Runs an invocation to the end, inside its scope and span.
async fn run_invocation(
    invocation: Arc<Invocation>,
    future: BoxFuture<'static, Result<Value, ActionError>>,
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
