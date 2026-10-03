//! Actions: async functions of a Thing, run as invocations.

use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use teta_wot_td::DataSchema;
use tokio::task::JoinError;

use crate::BoxFuture;
use crate::blob::Serialised;
use crate::cancel::Cancelled;
use crate::context::{ActionCtx, panic_message};
use crate::lock::GlobalLockBusy;
use crate::problem::ProblemDetails;
use crate::property::from_client;
use crate::thing::{DefinitionError, split_docstring};
use crate::validate::{SchemaValidator, ValidationError};

/// Default retention time for finished invocations.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(300);

/// An anticipated failure: logged without a cause chain. Create one
/// with [`ActionError::handled`].
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct HandledError(pub String);

/// The action panicked.
#[derive(Debug, Clone, thiserror::Error)]
#[error("The action panicked: {0}")]
pub struct Panicked(pub String);

/// Why an action failed.
///
/// Any error converts into it with `?`, like `anyhow::Error`. Some errors
/// get special treatment:
///
/// - [`Cancelled`] ends the invocation as `cancelled`;
/// - [`ActionError::handled`] is logged without its cause chain;
/// - [`GlobalLockBusy`] is the `GlobalLockBusyError`.
///
/// Everything else is logged at ERROR with its cause chain,.
pub struct ActionError {
    error: anyhow::Error,
    title: Cow<'static, str>,
}

impl<E> From<E> for ActionError
where
    E: Into<anyhow::Error> + 'static,
{
    fn from(error: E) -> Self {
        let title = short_type_name::<E>();
        let error = error.into();
        let title = if title == "Error" {
            // An `anyhow::Error` or similar: use the root cause's kind if known.
            Cow::Borrowed(known_title(&error).unwrap_or("Error"))
        } else {
            Cow::Borrowed(title)
        };
        Self { error, title }
    }
}

fn short_type_name<E>() -> &'static str {
    let name = std::any::type_name::<E>();
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit("::").next().unwrap_or(name)
}

fn known_title(error: &anyhow::Error) -> Option<&'static str> {
    if error.is::<Cancelled>() {
        Some("InvocationCancelledError")
    } else if error.is::<HandledError>() {
        Some("InvocationError")
    } else if error.is::<GlobalLockBusy>() {
        Some("GlobalLockBusyError")
    } else if error.is::<Panicked>() {
        Some("Panicked")
    } else {
        None
    }
}

impl fmt::Debug for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.error, f)
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.error)
    }
}

impl ActionError {
    /// An anticipated failure with a message for the user. It is logged at
    /// ERROR without a cause chain, and the invocation ends as `error`.
    pub fn handled(message: impl fmt::Display) -> Self {
        HandledError(message.to_string()).into()
    }

    /// Whether the action was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.error.is::<Cancelled>()
    }

    /// Whether this is an anticipated failure made with [`handled`](Self::handled).
    pub fn is_handled(&self) -> bool {
        self.error.is::<HandledError>()
    }

    /// Whether the action panicked.
    pub fn is_panic(&self) -> bool {
        self.error.is::<Panicked>()
    }

    /// The underlying error.
    pub fn as_anyhow(&self) -> &anyhow::Error {
        &self.error
    }

    /// The underlying error, for code outside actions that returns
    /// `anyhow::Result`: `?` can't convert an `ActionError` there, because
    /// it isn't a `std::error::Error`. Use
    /// `.map_err(ActionError::into_anyhow)?`.
    pub fn into_anyhow(self) -> anyhow::Error {
        self.error
    }

    /// The short name of the kind of error, used as the problem `title`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The cause chain, for the log's `traceback`.
    pub fn traceback(&self) -> String {
        format!("{:?}", self.error)
    }

    /// The problem details for this error.
    pub fn problem(&self) -> ProblemDetails {
        let detail = self.to_string();
        if self.is_cancelled() {
            ProblemDetails::teta_wot_things("InvocationCancelledError", detail, 500)
        } else if self.error.is::<GlobalLockBusy>() {
            ProblemDetails::teta_wot_things("GlobalLockBusyError", detail, 409)
        } else if self.is_handled() {
            ProblemDetails::teta_wot_things("InvocationError", detail, 500)
        } else {
            ProblemDetails::untyped(self.title.clone(), detail, 500)
        }
    }

    pub(crate) fn from_panic(payload: &(dyn std::any::Any + Send)) -> Self {
        Panicked(panic_message(payload)).into()
    }

    pub(crate) fn from_join(error: JoinError) -> Self {
        match error.try_into_panic() {
            Ok(payload) => Self::from_panic(&*payload),
            Err(error) => Panicked(error.to_string()).into(),
        }
    }
}

/// The input of an action that takes no parameters: a missing body, `null`
/// or `{}`, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoInput {
    was_object: bool,
}

impl Serialize for NoInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.was_object {
            serializer
                .serialize_map(Some(0))
                .and_then(serde::ser::SerializeMap::end)
        } else {
            serializer.serialize_none()
        }
    }
}

impl<'de> Deserialize<'de> for NoInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NoInputVisitor;

        impl<'de> Visitor<'de> for NoInputVisitor {
            type Value = NoInput;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("null or an empty object")
            }

            fn visit_unit<E: de::Error>(self) -> Result<NoInput, E> {
                Ok(NoInput { was_object: false })
            }

            fn visit_none<E: de::Error>(self) -> Result<NoInput, E> {
                Ok(NoInput { was_object: false })
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NoInput, A::Error> {
                match map.next_key::<String>()? {
                    None => Ok(NoInput { was_object: true }),
                    Some(key) => Err(de::Error::unknown_field(&key, &[])),
                }
            }
        }

        deserializer.deserialize_any(NoInputVisitor)
    }
}

impl JsonSchema for NoInput {
    fn schema_name() -> Cow<'static, str> {
        "StrictEmptyInput".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Represent the input of an action that never takes parameters.\n\nThis may be either an empty dictionary or ``None``.",
            "anyOf": [
                {
                    "description": "A model representing an object that must have no keys.",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                },
                {"type": "null"}
            ]
        })
    }
}

/// Metadata of an action.
#[derive(Debug, Clone)]
pub(crate) struct ActionMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub retention: Duration,
    pub global_lock: bool,
    pub semantic_types: Vec<String>,
}

impl Default for ActionMeta {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            retention: DEFAULT_RETENTION,
            global_lock: true,
            semantic_types: Vec::new(),
        }
    }
}

type Handler<T, I, O> =
    Arc<dyn Fn(Arc<T>, ActionCtx, I) -> BoxFuture<'static, Result<O, ActionError>> + Send + Sync>;

/// An action of a Thing type: an async function taking the Thing, an
/// [`ActionCtx`] and a typed input, returning a typed output.
///
/// The input type describes the action's parameters: usually a struct with
/// one field per parameter and `#[serde(deny_unknown_fields)]`. Use [`NoInput`] for an
/// action without parameters. The output can be any serialisable type; `()` becomes `null`.
#[must_use]
pub struct Action<T, I, O> {
    handler: Handler<T, I, O>,
    meta: ActionMeta,
    _types: PhantomData<fn(I) -> O>,
}

impl<T, I, O> Action<T, I, O>
where
    T: Send + Sync + 'static,
    I: DeserializeOwned + Serialize + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
{
    /// An action that runs `f`.
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(Arc<T>, ActionCtx, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, ActionError>> + Send + 'static,
    {
        Self {
            handler: Arc::new(move |thing, ctx, input| Box::pin(f(thing, ctx, input))),
            meta: ActionMeta::default(),
            _types: PhantomData,
        }
    }

    /// Sets the title (by default the action's name).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.meta.title = Some(title.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.meta.description = Some(description.into());
        self
    }

    /// Sets the title and description from a docstring.
    pub fn doc(mut self, doc: &str) -> Self {
        let (title, description) = split_docstring(doc);
        self.meta.title = Some(title);
        self.meta.description = Some(description);
        self
    }

    /// How long a finished invocation is kept (default 300 s).
    pub fn retention(mut self, retention: Duration) -> Self {
        self.meta.retention = retention;
        self
    }

    /// Opts out of holding the global lock while the action runs.
    pub fn global_lock(mut self, enabled: bool) -> Self {
        self.meta.global_lock = enabled;
        self
    }

    /// Adds a semantic annotation (`@type`).
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        self.meta.semantic_types.push(semantic_type.into());
        self
    }
}

/// The type-erased operations of one action of one Thing.
pub(crate) trait ActionHandler: Send + Sync {
    fn input_schema(&self) -> &DataSchema;
    fn output_schema(&self) -> &DataSchema;
    /// Validates the input and prepares the invocation's future. Returns the
    /// validated input (for the invocation's `input` field) and the future.
    fn prepare(
        &self,
        input: &Value,
        ctx: ActionCtx,
    ) -> Result<
        (
            Serialised,
            BoxFuture<'static, Result<Serialised, ActionError>>,
        ),
        ValidationError,
    >;
    /// Validates the input, and returns it coerced.
    fn validate(&self, input: &Value) -> Result<Value, ValidationError>;
}

struct TypedHandler<T, I, O> {
    thing: Arc<T>,
    handler: Handler<T, I, O>,
    input: SchemaValidator,
    output: DataSchema,
}

impl<T, I, O> ActionHandler for TypedHandler<T, I, O>
where
    T: Send + Sync + 'static,
    I: DeserializeOwned + Serialize + Send + 'static,
    O: Serialize + Send + 'static,
{
    fn input_schema(&self) -> &DataSchema {
        self.input.schema()
    }

    fn output_schema(&self) -> &DataSchema {
        &self.output
    }

    fn prepare(
        &self,
        input: &Value,
        ctx: ActionCtx,
    ) -> Result<
        (
            Serialised,
            BoxFuture<'static, Result<Serialised, ActionError>>,
        ),
        ValidationError,
    > {
        let typed: I = from_client(&self.input, input)?;
        let echo = Serialised::new(&typed).unwrap_or_default();
        let future = (self.handler)(Arc::clone(&self.thing), ctx, typed);
        Ok((
            echo,
            Box::pin(async move {
                let output = future.await?;
                Ok(Serialised::new(&output)?)
            }),
        ))
    }

    fn validate(&self, input: &Value) -> Result<Value, ValidationError> {
        let typed: I = from_client(&self.input, input)?;
        Ok(serde_json::to_value(&typed).unwrap_or(Value::Null))
    }
}

type BuildFn<T> =
    Box<dyn FnOnce(&Arc<T>, &str, &str) -> Result<Arc<dyn ActionHandler>, DefinitionError> + Send>;

/// An action ready to add to a [`ThingDefinition`](crate::ThingDefinition).
pub struct ActionSpec<T> {
    pub(crate) meta: ActionMeta,
    pub(crate) build: BuildFn<T>,
}

impl<T, I, O> From<Action<T, I, O>> for ActionSpec<T>
where
    T: Send + Sync + 'static,
    I: DeserializeOwned + Serialize + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
{
    fn from(action: Action<T, I, O>) -> Self {
        let handler = action.handler;
        ActionSpec {
            meta: action.meta,
            build: Box::new(move |thing, thing_name, name| {
                let error = |e: String| DefinitionError::new(thing_name, name, e);
                let input_schema = DataSchema::for_type::<I>().map_err(|e| error(e.to_string()))?;
                let input = SchemaValidator::new(input_schema).map_err(|e| error(e.to_string()))?;
                let output = DataSchema::for_type::<O>().map_err(|e| error(e.to_string()))?;
                Ok(Arc::new(TypedHandler {
                    thing: Arc::clone(thing),
                    handler,
                    input,
                    output,
                }) as Arc<dyn ActionHandler>)
            }),
        }
    }
}
