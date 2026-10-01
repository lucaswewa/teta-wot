//! Properties: data properties backed by [`Prop<T>`] cells, and functional
//! properties backed by async getters and setters.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use teta_wot_td::{Constraints, DataSchema};
use tokio::sync::watch;

use crate::BoxFuture;
use crate::broker::{Message, MessageBroker, MessageKind};
use crate::cancel::Cancelled;
use crate::context::InvocationScope;
use crate::device::DeviceError;
use crate::lock::GlobalLockBusy;
use crate::problem::ProblemDetails;
use crate::runtime::Shared;
use crate::thing::{DefinitionError, split_docstring};
use crate::validate::{LocItem, SchemaValidator, ValidationError};

/// What a property value needs: JSON (de)serialisation, a JSON Schema,
/// cloning, and sharing between threads.
pub trait PropValue:
    Serialize + DeserializeOwned + JsonSchema + Clone + Send + Sync + 'static
{
}

impl<T> PropValue for T where
    T: Serialize + DeserializeOwned + JsonSchema + Clone + Send + Sync + 'static
{
}

/// A property operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PropertyError {
    /// The value doesn't validate (HTTP 422).
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    /// Clients may not write the property.
    #[error("property `{0}` is read-only")]
    ReadOnly(String),
    /// The property can't be reset.
    #[error("property `{0}` can't be reset")]
    NotResettable(String),
    /// The global lock is held by someone else (HTTP 409).
    #[error(transparent)]
    GlobalLockBusy(#[from] GlobalLockBusy),
    /// A device call failed.
    #[error(transparent)]
    Device(#[from] DeviceError),
    /// The operation was cancelled.
    #[error(transparent)]
    Cancelled(#[from] Cancelled),
    /// The property is defined in a way that can't work, for example a
    /// constraint that doesn't suit its type.
    #[error("the property is badly defined: {0}")]
    Definition(String),
    /// A getter, setter or resetter failed.
    #[error("{0:#}")]
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for PropertyError {
    fn from(error: anyhow::Error) -> Self {
        PropertyError::Failed(error)
    }
}

impl PropertyError {
    /// Wraps any error from a getter, setter or resetter.
    pub fn failed(error: impl Into<anyhow::Error>) -> Self {
        PropertyError::Failed(error.into())
    }

    /// The problem details for this error.
    pub fn problem(&self) -> ProblemDetails {
        match self {
            PropertyError::GlobalLockBusy(busy) => {
                ProblemDetails::teta_wot_things("GlobalLockBusyError", busy.to_string(), 409)
            }
            PropertyError::Cancelled(c) => {
                ProblemDetails::teta_wot_things("InvocationCancelledError", c.to_string(), 500)
            }
            other => ProblemDetails::untyped(error_title(other), format!("{other}"), 500),
        }
    }
}

fn error_title(error: &PropertyError) -> &'static str {
    match error {
        PropertyError::Invalid(_) => "ValidationError",
        PropertyError::ReadOnly(_) => "ReadOnlyPropertyError",
        PropertyError::NotResettable(_) => "FeatureNotAvailableError",
        PropertyError::Device(_) => "DeviceError",
        PropertyError::Definition(_) => "PropertyDefinitionError",
        _ => "PropertyError",
    }
}

/// Where a bound [`Prop`] publishes its changes.
struct Binding {
    thing: Arc<str>,
    name: Arc<str>,
    broker: Arc<MessageBroker>,
}

/// The value of a data property: a typed cell with validation and change
/// notification.
///
/// The Thing owns its `Prop`s and reads and writes them from Rust with
/// [`get`](Self::get) and [`set`](Self::set). `set` checks the property's
/// constraints (the type is checked by the compiler), stores the value and
/// notifies observers: Rust code through [`subscribe`](Self::subscribe),
/// and clients through the server's message broker. `read_only` on the
/// property only restricts clients; Rust code can always set it.
pub struct Prop<T: PropValue> {
    value: watch::Sender<T>,
    default: T,
    constraints: Constraints,
    validator: OnceLock<Result<SchemaValidator, String>>,
    binding: OnceLock<Binding>,
}

impl<T: PropValue + fmt::Debug> fmt::Debug for Prop<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prop")
            .field("value", &*self.value.borrow())
            .field("default", &self.default)
            .finish_non_exhaustive()
    }
}

impl<T: PropValue + Default> Default for Prop<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: PropValue> Prop<T> {
    /// A cell holding `default`, which is also the value it resets to.
    pub fn new(default: T) -> Self {
        Self {
            value: watch::Sender::new(default.clone()),
            default,
            constraints: Constraints::new(),
            validator: OnceLock::new(),
            binding: OnceLock::new(),
        }
    }

    /// Adds value constraints (`ge`, `lt`, `pattern`, …). They are checked
    /// on every write and appear in the Thing Description.
    pub fn with_constraints(mut self, constraints: Constraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// The constraints.
    pub fn constraints(&self) -> &Constraints {
        &self.constraints
    }

    /// The default value.
    pub fn default_value(&self) -> &T {
        &self.default
    }

    /// A copy of the current value.
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// Reads the current value without copying it. Don't hold on to the
    /// borrow: writers wait for it.
    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.value.borrow())
    }

    /// Checks the constraints, stores `value` and notifies observers.
    pub fn set(&self, value: T) -> Result<(), PropertyError> {
        if !self.constraints.is_empty() {
            let json = serde_json::to_value(&value).map_err(PropertyError::failed)?;
            self.validator()?.validate(&json, &[])?;
        }
        self.store(value);
        Ok(())
    }

    /// Changes the value in place, then checks and stores it like
    /// [`set`](Self::set). If the new value is invalid nothing changes.
    pub fn update(&self, f: impl FnOnce(&mut T)) -> Result<(), PropertyError> {
        let mut value = self.get();
        f(&mut value);
        self.set(value)
    }

    /// Sets the default value again.
    pub fn reset(&self) -> Result<(), PropertyError> {
        self.set(self.default.clone())
    }

    /// Watches the value. The receiver sees the latest value; values set in
    /// quick succession may be seen only once.
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.value.subscribe()
    }

    /// The property's DataSchema: its type's, with the constraints applied.
    pub fn data_schema(&self) -> Result<DataSchema, PropertyError> {
        Ok(self.validator()?.schema().clone())
    }

    fn validator(&self) -> Result<&SchemaValidator, PropertyError> {
        self.validator
            .get_or_init(|| build_validator::<T>(&self.constraints))
            .as_ref()
            .map_err(|e| PropertyError::Definition(e.clone()))
    }

    fn store(&self, value: T) {
        if let Some(binding) = self.binding.get()
            && binding
                .broker
                .has_subscribers(&binding.thing, &binding.name)
            && let Ok(payload) = serde_json::to_value(&value)
        {
            binding.broker.publish(Message {
                thing: binding.thing.to_string(),
                affordance: binding.name.to_string(),
                kind: MessageKind::Property,
                payload,
            });
        }
        self.value.send_replace(value);
    }

    /// Validates a client's value (pydantic-style, located under `body`),
    /// converts it and sets it.
    fn set_from_client(&self, value: &Value) -> Result<(), PropertyError> {
        let typed = from_client::<T>(self.validator()?, value)?;
        self.store(typed);
        Ok(())
    }

    fn bind(&self, binding: Binding) {
        let _ = self.binding.set(binding);
    }
}

pub(crate) fn build_validator<T: JsonSchema>(
    constraints: &Constraints,
) -> Result<SchemaValidator, String> {
    let mut schema = DataSchema::for_type::<T>().map_err(|e| e.to_string())?;
    constraints.apply(&mut schema).map_err(|e| e.to_string())?;
    SchemaValidator::new(schema).map_err(|e| e.to_string())
}

/// Validates and converts a value sent by a client.
pub(crate) fn from_client<T: DeserializeOwned>(
    validator: &SchemaValidator,
    value: &Value,
) -> Result<T, ValidationError> {
    let loc = vec![LocItem::from("body")];
    let coerced = validator.validate(value, &loc)?;
    serde_json::from_value(coerced.clone())
        .map_err(|e| ValidationError::from_serde(&e, loc, coerced))
}

/// Metadata shared by both kinds of property.
#[derive(Debug, Clone, Default)]
pub(crate) struct PropertyMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub read_only: Option<bool>,
    pub global_lock: Option<bool>,
    pub unit: Option<String>,
    pub semantic_types: Vec<String>,
}

macro_rules! meta_setters {
    () => {
        /// Sets the title (by default the property's name).
        pub fn title(mut self, title: impl Into<String>) -> Self {
            self.meta.title = Some(title.into());
            self
        }

        /// Sets the description.
        pub fn description(mut self, description: impl Into<String>) -> Self {
            self.meta.description = Some(description.into());
            self
        }

        /// Sets the title and description from a docstring:
        /// the title is the first line, and the description is the
        /// rest if the second line is blank, otherwise the whole text.
        pub fn doc(mut self, doc: &str) -> Self {
            let (title, description) = split_docstring(doc);
            self.meta.title = Some(title);
            self.meta.description = Some(description);
            self
        }

        /// Opts out of the global lock for client writes.
        pub fn global_lock(mut self, enabled: bool) -> Self {
            self.meta.global_lock = Some(enabled);
            self
        }

        /// Sets the unit of measure shown in the Thing Description.
        pub fn unit(mut self, unit: impl Into<String>) -> Self {
            self.meta.unit = Some(unit.into());
            self
        }

        /// Adds a semantic annotation (`@type`) shown in the Thing Description.
        pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
            self.meta.semantic_types.push(semantic_type.into());
            self
        }
    };
}

/// The type-erased operations of one property of one Thing.
pub(crate) trait PropertyHandler: Send + Sync {
    fn read(&self) -> BoxFuture<'_, Result<Value, PropertyError>>;
    fn write(&self, value: Value) -> BoxFuture<'_, Result<(), PropertyError>>;
    fn resettable(&self) -> bool;
    /// Only called when `resettable()` is true.
    fn reset(&self) -> BoxFuture<'_, Result<(), PropertyError>>;
    fn default_json(&self) -> Option<Value>;
    fn schema(&self) -> &DataSchema;
    fn has_setter(&self) -> bool;
    fn observable(&self) -> bool;
}

type BuildFn<T> = Box<
    dyn FnOnce(
            &Arc<T>,
            &str,
            &str,
            &Arc<Shared>,
        ) -> Result<Arc<dyn PropertyHandler>, DefinitionError>
        + Send,
>;

/// A property of a Thing type, ready to add to a
/// [`ThingDefinition`](crate::ThingDefinition). Made from a
/// [`DataProperty`] or a [`FunctionalProperty`].
pub struct PropertySpec<T> {
    pub(crate) meta: PropertyMeta,
    pub(crate) default_read_only: bool,
    pub(crate) build: BuildFn<T>,
}

/// A data property: a [`Prop`] field of the Thing.
///
/// Data properties are writable by clients unless marked
/// [`read_only`](Self::read_only), resettable to their default, and
/// observable.
#[must_use]
pub struct DataProperty<T, V: PropValue> {
    accessor: fn(&T) -> &Prop<V>,
    meta: PropertyMeta,
}

impl<T: Send + Sync + 'static, V: PropValue> DataProperty<T, V> {
    /// The property stored in the field that `accessor` returns, for
    /// example `|t: &Counter| &t.count`.
    pub fn new(accessor: fn(&T) -> &Prop<V>) -> Self {
        Self {
            accessor,
            meta: PropertyMeta::default(),
        }
    }

    /// Clients may read but not write the property.
    pub fn read_only(mut self) -> Self {
        self.meta.read_only = Some(true);
        self
    }

    meta_setters!();
}

struct DataHandler<T, V: PropValue> {
    thing: Arc<T>,
    accessor: fn(&T) -> &Prop<V>,
    schema: DataSchema,
}

impl<T: Send + Sync + 'static, V: PropValue> DataHandler<T, V> {
    fn prop(&self) -> &Prop<V> {
        (self.accessor)(&self.thing)
    }
}

impl<T: Send + Sync + 'static, V: PropValue> PropertyHandler for DataHandler<T, V> {
    fn read(&self) -> BoxFuture<'_, Result<Value, PropertyError>> {
        let value = self.prop().read(|v| serde_json::to_value(v));
        Box::pin(async move { value.map_err(PropertyError::failed) })
    }

    fn write(&self, value: Value) -> BoxFuture<'_, Result<(), PropertyError>> {
        let result = self.prop().set_from_client(&value);
        Box::pin(async move { result })
    }

    fn resettable(&self) -> bool {
        true
    }

    fn reset(&self) -> BoxFuture<'_, Result<(), PropertyError>> {
        Box::pin(async move { self.prop().reset() })
    }

    fn default_json(&self) -> Option<Value> {
        serde_json::to_value(self.prop().default_value()).ok()
    }

    fn schema(&self) -> &DataSchema {
        &self.schema
    }

    fn has_setter(&self) -> bool {
        true
    }

    fn observable(&self) -> bool {
        true
    }
}

impl<T: Send + Sync + 'static, V: PropValue> From<DataProperty<T, V>> for PropertySpec<T> {
    fn from(property: DataProperty<T, V>) -> Self {
        let accessor = property.accessor;
        PropertySpec {
            meta: property.meta,
            default_read_only: false,
            build: Box::new(move |thing, thing_name, name, shared| {
                let prop = accessor(thing);
                let schema = prop
                    .data_schema()
                    .map_err(|e| DefinitionError::new(thing_name, name, e.to_string()))?;
                prop.bind(Binding {
                    thing: thing_name.into(),
                    name: name.into(),
                    broker: Arc::clone(&shared.broker),
                });
                Ok(Arc::new(DataHandler {
                    thing: Arc::clone(thing),
                    accessor,
                    schema,
                }) as Arc<dyn PropertyHandler>)
            }),
        }
    }
}

type Getter<T, V> =
    Arc<dyn Fn(Arc<T>) -> BoxFuture<'static, Result<V, PropertyError>> + Send + Sync>;
type Setter<T, V> =
    Arc<dyn Fn(Arc<T>, V) -> BoxFuture<'static, Result<(), PropertyError>> + Send + Sync>;
type Resetter<T> =
    Arc<dyn Fn(Arc<T>) -> BoxFuture<'static, Result<(), PropertyError>> + Send + Sync>;

/// A functional property: its value comes from a getter, and writes go to a
/// setter, both async methods of the Thing.
///
/// It is read-only for clients unless it has a setter, and it isn't
/// observable. It can be reset if it has a resetter, or a default and a
/// setter.
#[must_use]
pub struct FunctionalProperty<T, V: PropValue> {
    getter: Getter<T, V>,
    setter: Option<Setter<T, V>>,
    resetter: Option<Resetter<T>>,
    default: Option<V>,
    constraints: Constraints,
    meta: PropertyMeta,
}

impl<T: Send + Sync + 'static, V: PropValue> FunctionalProperty<T, V> {
    /// A property whose value `getter` returns.
    pub fn getter<F, Fut>(getter: F) -> Self
    where
        F: Fn(Arc<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<V, PropertyError>> + Send + 'static,
    {
        Self {
            getter: Arc::new(move |t| Box::pin(getter(t))),
            setter: None,
            resetter: None,
            default: None,
            constraints: Constraints::new(),
            meta: PropertyMeta::default(),
        }
    }

    /// A property whose value a blocking function returns. It runs on
    /// tokio's blocking thread pool, so it may do slow synchronous work.
    pub fn blocking_getter<F>(getter: F) -> Self
    where
        F: Fn(&T) -> Result<V, PropertyError> + Send + Sync + 'static,
    {
        let getter = Arc::new(getter);
        Self::getter(move |t: Arc<T>| {
            let getter = Arc::clone(&getter);
            async move {
                let span = tracing::Span::current();
                let scope = InvocationScope::current();
                tokio::task::spawn_blocking(move || {
                    let _entered = span.enter();
                    match scope {
                        Some(scope) => scope.run_blocking(|| getter(&t)),
                        None => getter(&t),
                    }
                })
                .await
                .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
            }
        })
    }

    /// Adds a setter, which makes the property writable by clients.
    pub fn setter<F, Fut>(mut self, setter: F) -> Self
    where
        F: Fn(Arc<T>, V) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), PropertyError>> + Send + 'static,
    {
        self.setter = Some(Arc::new(move |t, v| Box::pin(setter(t, v))));
        self
    }

    /// Adds a resetter, which puts the property back to a default state.
    pub fn resetter<F, Fut>(mut self, resetter: F) -> Self
    where
        F: Fn(Arc<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), PropertyError>> + Send + 'static,
    {
        self.resetter = Some(Arc::new(move |t| Box::pin(resetter(t))));
        self
    }

    /// Sets the default value shown in the Thing Description. With a setter,
    /// it also makes the property resettable.
    pub fn default_value(mut self, default: V) -> Self {
        self.default = Some(default);
        self
    }

    /// Adds value constraints, checked before the setter runs.
    pub fn constraints(mut self, constraints: Constraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// Clients may not write the property, even though it has a setter.
    pub fn read_only(mut self) -> Self {
        self.meta.read_only = Some(true);
        self
    }

    meta_setters!();
}

struct FunctionalHandler<T, V: PropValue> {
    thing: Arc<T>,
    getter: Getter<T, V>,
    setter: Option<Setter<T, V>>,
    resetter: Option<Resetter<T>>,
    default: Option<V>,
    validator: SchemaValidator,
}

impl<T: Send + Sync + 'static, V: PropValue> PropertyHandler for FunctionalHandler<T, V> {
    fn read(&self) -> BoxFuture<'_, Result<Value, PropertyError>> {
        let future = (self.getter)(Arc::clone(&self.thing));
        Box::pin(async move {
            let value = future.await?;
            serde_json::to_value(value).map_err(PropertyError::failed)
        })
    }

    fn write(&self, value: Value) -> BoxFuture<'_, Result<(), PropertyError>> {
        Box::pin(async move {
            let Some(setter) = &self.setter else {
                return Err(PropertyError::ReadOnly(String::new()));
            };
            let typed = from_client::<V>(&self.validator, &value)?;
            setter(Arc::clone(&self.thing), typed).await
        })
    }

    fn resettable(&self) -> bool {
        self.resetter.is_some() || (self.setter.is_some() && self.default.is_some())
    }

    fn reset(&self) -> BoxFuture<'_, Result<(), PropertyError>> {
        Box::pin(async move {
            if let Some(resetter) = &self.resetter {
                return resetter(Arc::clone(&self.thing)).await;
            }
            match (&self.setter, &self.default) {
                (Some(setter), Some(default)) => {
                    setter(Arc::clone(&self.thing), default.clone()).await
                }
                _ => Err(PropertyError::NotResettable(String::new())),
            }
        })
    }

    fn default_json(&self) -> Option<Value> {
        self.default
            .as_ref()
            .and_then(|d| serde_json::to_value(d).ok())
    }

    fn schema(&self) -> &DataSchema {
        self.validator.schema()
    }

    fn has_setter(&self) -> bool {
        self.setter.is_some()
    }

    fn observable(&self) -> bool {
        false
    }
}

impl<T: Send + Sync + 'static, V: PropValue> From<FunctionalProperty<T, V>> for PropertySpec<T> {
    fn from(property: FunctionalProperty<T, V>) -> Self {
        let default_read_only = property.setter.is_none();
        let FunctionalProperty {
            getter,
            setter,
            resetter,
            default,
            constraints,
            meta,
        } = property;
        PropertySpec {
            meta,
            default_read_only,
            build: Box::new(move |thing, thing_name, name, _shared| {
                let validator = build_validator::<V>(&constraints)
                    .map_err(|e| DefinitionError::new(thing_name, name, e))?;
                Ok(Arc::new(FunctionalHandler {
                    thing: Arc::clone(thing),
                    getter,
                    setter,
                    resetter,
                    default,
                    validator,
                }) as Arc<dyn PropertyHandler>)
            }),
        }
    }
}

/// One property of a running Thing: what clients (and the HTTP binding) use.
#[derive(Clone)]
pub struct PropertyEntry {
    pub(crate) name: String,
    pub(crate) thing: Arc<str>,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) read_only: bool,
    pub(crate) global_lock: bool,
    pub(crate) unit: Option<String>,
    pub(crate) semantic_types: Vec<String>,
    pub(crate) handler: Arc<dyn PropertyHandler>,
    pub(crate) shared: Arc<Shared>,
}

impl fmt::Debug for PropertyEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PropertyEntry")
            .field("thing", &self.thing)
            .field("name", &self.name)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl PropertyEntry {
    /// The property's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The property's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The property's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Whether clients may only read the property.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether clients may observe the property (data properties only).
    pub fn is_observable(&self) -> bool {
        self.handler.observable()
    }

    /// Whether clients may reset the property. Read-only properties can't be
    /// reset by clients (reference bug B4 fixed).
    pub fn is_resettable(&self) -> bool {
        !self.read_only && self.handler.resettable()
    }

    /// The property's DataSchema, with its constraints.
    pub fn data_schema(&self) -> &DataSchema {
        self.handler.schema()
    }

    /// The default value, if the property has one.
    pub fn default_value(&self) -> Option<Value> {
        self.handler.default_json()
    }

    /// The unit of measure, if set.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }

    /// Semantic annotations.
    pub fn semantic_types(&self) -> &[String] {
        &self.semantic_types
    }

    /// Reads the value. Reads don't take the global lock.
    pub async fn read(&self) -> Result<Value, PropertyError> {
        self.handler.read().await
    }

    /// Writes a value sent by a client: refuses read-only properties, takes
    /// the global lock if enabled, validates the value (errors located under
    /// `body`) and stores it.
    ///
    /// `null` counts as a missing body and is refused with a `missing` error,
    /// even for a property that may be `null`.
    pub async fn write(&self, value: Value) -> Result<(), PropertyError> {
        if self.read_only || !self.handler.has_setter() {
            return Err(PropertyError::ReadOnly(self.name.clone()));
        }
        if value.is_null() {
            return Err(ValidationError::missing(vec![LocItem::from("body")], Value::Null).into());
        }
        let _guard = self.lock().await?;
        self.handler.write(value).await
    }

    /// Resets the property for a client: refuses read-only properties, takes
    /// the global lock if enabled, and resets.
    pub async fn reset(&self) -> Result<(), PropertyError> {
        if self.read_only {
            return Err(PropertyError::ReadOnly(self.name.clone()));
        }
        if !self.handler.resettable() {
            return Err(PropertyError::NotResettable(self.name.clone()));
        }
        let _guard = self.lock().await?;
        self.handler.reset().await
    }

    async fn lock(&self) -> Result<Option<crate::lock::GlobalLockGuard>, GlobalLockBusy> {
        match (&self.shared.lock, self.global_lock) {
            (Some(lock), true) => {
                let owner =
                    InvocationScope::current().map_or_else(uuid::Uuid::new_v4, |s| s.lock_owner());
                lock.acquire(owner).await.map(Some)
            }
            _ => Ok(None),
        }
    }
}
