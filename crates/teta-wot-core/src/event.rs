//! Events: [`Event<T>`] fields that a Thing emits, and their definitions.
//!
//! An event is a field of the
//! Thing, declared with [`EventSpec`] (or `#[event]`); emitting it sends the
//! data to its subscribers: clients over WebSocket and SSE, through the
//! server's [`MessageBroker`], and Rust code through
//! [`Event::subscribe`]. The TD describes the data with `T`'s schema.

use std::fmt;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::Serialize;
use teta_wot_td::{DataSchema, DataType};
use tokio::sync::broadcast;

use crate::broker::{Message, MessageBroker, MessageKind};
use crate::thing::{DefinitionError, split_docstring};

/// A type that can be an event's data: serialisable, described by a JSON
/// Schema, and cheap enough to clone for each Rust subscriber.
pub trait EventData: Serialize + JsonSchema + Clone + Send + Sync + 'static {}

impl<T: Serialize + JsonSchema + Clone + Send + Sync + 'static> EventData for T {}

/// How many events a Rust subscriber can fall behind before it misses some.
pub const LOCAL_EVENT_BUFFER: usize = 16;

/// Where a bound event publishes.
struct EventBinding {
    thing: Arc<str>,
    name: Arc<str>,
    broker: Arc<MessageBroker>,
}

struct EventShared<T> {
    binding: OnceLock<EventBinding>,
    local: broadcast::Sender<T>,
}

/// An event that a Thing emits: a field of the Thing, declared as an
/// affordance with `#[event]` or [`EventSpec`].
///
/// `Event` is a cheap handle: clones emit the same event. Clone it into a
/// device's thread, a spawned task or a callback, and emit from there:
/// [`emit`](Self::emit) never waits and needs no async runtime.
///
/// ```
/// use teta_wot_core::Event;
///
/// let tripped: Event<String> = Event::new();
/// let mut rust_side = tripped.subscribe();
/// let from_thread = tripped.clone();
/// std::thread::spawn(move || from_thread.emit("door open".to_owned()))
///     .join()
///     .unwrap();
/// assert_eq!(rust_side.try_recv().unwrap(), "door open");
/// ```
pub struct Event<T: EventData> {
    shared: Arc<EventShared<T>>,
}

impl<T: EventData> Event<T> {
    /// An event with no subscribers yet.
    pub fn new() -> Self {
        let (local, _) = broadcast::channel(LOCAL_EVENT_BUFFER);
        Self {
            shared: Arc::new(EventShared {
                binding: OnceLock::new(),
                local,
            }),
        }
    }

    /// Sends `data` to every subscriber, without waiting. Subscribers whose
    /// buffers are full miss it.
    pub fn emit(&self, data: T) {
        if let Some(binding) = self.shared.binding.get()
            && binding
                .broker
                .has_subscribers(&binding.thing, &binding.name)
        {
            match serde_json::to_value(&data) {
                Ok(payload) => binding.broker.publish(Message::new(
                    &*binding.thing,
                    &*binding.name,
                    MessageKind::Event,
                    payload,
                )),
                Err(error) => tracing::error!(
                    target: "wot::things",
                    thing = %binding.thing,
                    "can't send event `{}`: its data doesn't serialise: {error}",
                    binding.name,
                ),
            }
        }
        // No Rust subscribers isn't an error.
        let _ = self.shared.local.send(data);
    }

    /// Receives the events emitted from now on, in Rust. A receiver that
    /// falls more than [`LOCAL_EVENT_BUFFER`] events behind misses the
    /// oldest ones (`RecvError::Lagged`).
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.shared.local.subscribe()
    }

    /// The schema of the event's data, or `None` for `()`.
    pub fn data_schema() -> Result<Option<DataSchema>, String> {
        let schema = DataSchema::for_type::<T>().map_err(|e| e.to_string())?;
        let is_null = DataSchema {
            data_type: Some(DataType::Null),
            ..DataSchema::default()
        };
        Ok((schema != is_null).then_some(schema))
    }

    fn bind(&self, binding: EventBinding) {
        let _ = self.shared.binding.set(binding);
    }
}

impl<T: EventData> Default for Event<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: EventData> Clone for Event<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T: EventData> fmt::Debug for Event<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let binding = self.shared.binding.get();
        f.debug_struct("Event")
            .field("thing", &binding.map(|b| &*b.thing))
            .field("name", &binding.map(|b| &*b.name))
            .finish_non_exhaustive()
    }
}

/// Metadata of an event.
#[derive(Debug, Clone, Default)]
pub(crate) struct EventMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub semantic_types: Vec<String>,
}

type EventBuild<T> = Box<
    dyn FnOnce(
            &Arc<T>,
            &str,
            &str,
            &Arc<MessageBroker>,
        ) -> Result<Option<DataSchema>, DefinitionError>
        + Send,
>;

/// The definition of an event affordance: which [`Event`] field of the
/// Thing it is, and its title and description.
///
/// ```
/// use teta_wot_core::{Event, EventSpec, Thing, ThingDefinition};
///
/// struct Door {
///     opened: Event<String>,
/// }
///
/// impl Thing for Door {
///     fn definition() -> ThingDefinition<Self> {
///         ThingDefinition::new("Door")
///             .event("opened", EventSpec::new(|t: &Door| &t.opened).doc("The door opened."))
///     }
/// }
/// ```
pub struct EventSpec<T> {
    pub(crate) meta: EventMeta,
    pub(crate) build: EventBuild<T>,
}

impl<T: Send + Sync + 'static> EventSpec<T> {
    /// The event held in a field of the Thing.
    pub fn new<V: EventData>(accessor: fn(&T) -> &Event<V>) -> Self {
        Self {
            meta: EventMeta::default(),
            build: Box::new(move |thing, thing_name, name, broker| {
                let event = accessor(thing);
                let schema = Event::<V>::data_schema()
                    .map_err(|e| DefinitionError::new(thing_name, name, e))?;
                event.bind(EventBinding {
                    thing: thing_name.into(),
                    name: name.into(),
                    broker: Arc::clone(broker),
                });
                Ok(schema)
            }),
        }
    }

    /// Sets the title (by default the event's name).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.meta.title = Some(title.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.meta.description = Some(description.into());
        self
    }

    /// Sets the title and description from a docstring, by the same rule as
    /// properties and actions.
    pub fn doc(mut self, doc: &str) -> Self {
        let (title, description) = split_docstring(doc);
        self.meta.title = Some(title);
        self.meta.description = Some(description);
        self
    }

    /// Adds a semantic annotation (`@type`) shown in the Thing Description.
    pub fn semantic_type(mut self, semantic_type: impl Into<String>) -> Self {
        self.meta.semantic_types.push(semantic_type.into());
        self
    }
}

impl<T> fmt::Debug for EventSpec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSpec")
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// One event of a running Thing, as clients see it.
#[derive(Debug, Clone)]
pub struct EventEntry {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) semantic_types: Vec<String>,
    pub(crate) data: Option<DataSchema>,
}

impl EventEntry {
    /// The event's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The description, if any.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The schema of the event's data (`None` for events without data).
    pub fn data_schema(&self) -> Option<&DataSchema> {
        self.data.as_ref()
    }

    /// The semantic annotations.
    pub fn semantic_types(&self) -> &[String] {
        &self.semantic_types
    }
}
