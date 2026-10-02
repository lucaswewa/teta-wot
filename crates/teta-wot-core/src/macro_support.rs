//! Support for the code that `wot-macros` generates. Not a
//! public API: it is exported as `wot_core::__private` for the macros, and
//! may change at any time.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::device::Driver;
use crate::property::PropValue;
use crate::thing::{Thing, ThingCtx, ThingDefinition};

pub use anyhow;
pub use schemars;
pub use serde;
pub use serde_json;

/// A boxed, sendable future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// What `#[thing_impl]` adds to a Thing: the affordances defined by
/// methods, and the lifecycle hooks.
pub trait ThingMethods: Thing {
    /// Adds the method affordances to the definition built from the fields.
    fn methods(definition: ThingDefinition<Self>) -> ThingDefinition<Self>;

    /// The `#[on_start]` hook.
    fn start(self: Arc<Self>, ctx: ThingCtx) -> BoxFuture<anyhow::Result<()>> {
        let _ = ctx;
        Box::pin(async { Ok(()) })
    }

    /// The `#[on_stop]` hook.
    fn stop(self: Arc<Self>, ctx: ThingCtx) -> BoxFuture<()> {
        let _ = ctx;
        Box::pin(async {})
    }
}

/// Finds a Thing's [`ThingMethods`], if it has any, by autoref
/// specialisation: `(&Methods::<T>::new()).definition(d)` resolves to
/// [`ViaImpl`] when `T: ThingMethods`, and to [`ViaDefault`] otherwise. So a
/// `#[derive(Thing)]` struct doesn't need a `#[thing_impl]` block. It only
/// works for concrete types, which is why Things can't be generic.
pub struct Methods<T>(PhantomData<fn() -> T>);

impl<T> Methods<T> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

/// The dispatch when the Thing has a `#[thing_impl]` block.
pub trait ViaImpl<T: Thing> {
    fn definition(&self, definition: ThingDefinition<T>) -> ThingDefinition<T>;
    fn start(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<anyhow::Result<()>>;
    fn stop(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<()>;
}

impl<T: ThingMethods> ViaImpl<T> for Methods<T> {
    fn definition(&self, definition: ThingDefinition<T>) -> ThingDefinition<T> {
        T::methods(definition)
    }

    fn start(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<anyhow::Result<()>> {
        ThingMethods::start(thing, ctx)
    }

    fn stop(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<()> {
        ThingMethods::stop(thing, ctx)
    }
}

/// The dispatch when the Thing has no `#[thing_impl]` block.
pub trait ViaDefault<T: Thing> {
    fn definition(&self, definition: ThingDefinition<T>) -> ThingDefinition<T>;
    fn start(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<anyhow::Result<()>>;
    fn stop(&self, thing: Arc<T>, ctx: ThingCtx) -> BoxFuture<()>;
}

impl<T: Thing> ViaDefault<T> for &Methods<T> {
    fn definition(&self, definition: ThingDefinition<T>) -> ThingDefinition<T> {
        definition
    }

    fn start(&self, _: Arc<T>, _: ThingCtx) -> BoxFuture<anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn stop(&self, _: Arc<T>, _: ThingCtx) -> BoxFuture<()> {
        Box::pin(async {})
    }
}

// Assertions placed on the user's types, so that a type that can't be used
// is reported where it is written, in words about Things.

/// A type that can be a property's value.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be the value of a property",
    label = "not usable as a property value",
    note = "a property value must implement `Serialize`, `Deserialize`, `JsonSchema`, `Clone`, `Send` and `Sync`"
)]
pub trait PropertyValue {}

#[diagnostic::do_not_recommend]
impl<T: PropValue> PropertyValue for T {}

/// A type that can be an action's parameter.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be the type of an action parameter",
    label = "not usable as an action parameter",
    note = "an action parameter must implement `Serialize`, `Deserialize`, `JsonSchema` and `Send`, and own its data"
)]
pub trait ActionParameter {}

#[diagnostic::do_not_recommend]
impl<T: Serialize + DeserializeOwned + JsonSchema + Send + 'static> ActionParameter for T {}

/// A type that an action can return.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be the output of an action",
    label = "not usable as an action output",
    note = "an action output must implement `Serialize`, `JsonSchema` and `Send`"
)]
pub trait ActionOutput {}

#[diagnostic::do_not_recommend]
impl<T: Serialize + JsonSchema + Send + 'static> ActionOutput for T {}

/// A type that can be an event's data.
#[diagnostic::on_unimplemented(
    message = "`{Self}` can't be the data of an event",
    label = "not usable as event data",
    note = "event data must implement `Serialize`, `JsonSchema`, `Clone`, `Send` and `Sync`, and own its data"
)]
pub trait EventPayload {}

#[diagnostic::do_not_recommend]
impl<T: crate::EventData> EventPayload for T {}

pub fn property_value<T: PropertyValue + ?Sized>() {}
pub fn event_data<T: EventPayload + ?Sized>() {}
pub fn action_parameter<T: ActionParameter + ?Sized>() {}
pub fn action_output<T: ActionOutput + ?Sized>() {}

/// The value of a `#[device(init = …)]` expression: a driver, or a result
/// with one.
pub trait IntoDriver<D> {
    fn into_driver(self) -> anyhow::Result<D>;
}

impl<D: Driver> IntoDriver<D> for D {
    fn into_driver(self) -> anyhow::Result<D> {
        Ok(self)
    }
}

impl<D: Driver, E: Into<anyhow::Error>> IntoDriver<D> for Result<D, E> {
    fn into_driver(self) -> anyhow::Result<D> {
        self.map_err(Into::into)
    }
}
