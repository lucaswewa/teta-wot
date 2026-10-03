//! In-process calls between Things.
//!
//! A [`ThingRef<T>`] is a typed reference to a running Thing. Calling its
//! affordances through it applies what a client's request would get:
//! validation, the global lock and read-only rules. Calls run in the caller's
//! invocation context, so cancelling the caller cancels them, and their log
//! lines go to the caller's invocation.
//!
//! `#[thing_impl]` generates a trait, `{Thing}Actions`, with one typed
//! method per action, implemented for `ThingRef<{Thing}>`. `ThingRef`'s own
//! operations are associated functions (`ThingRef::thing(&r)`), like `Arc`'s,
//! so they never hide an action's method.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::action::ActionError;
use crate::context::ActionCtx;
use crate::property::PropertyError;
use crate::runtime::{ActionEntry, ThingHandle};
use crate::thing::Thing;

/// A typed reference to a running Thing, for in-process calls.
///
/// Get one from [`Runtime::thing_ref`](crate::Runtime::thing_ref) (and, in
/// Phase 5, from a slot).
pub struct ThingRef<T> {
    handle: Arc<ThingHandle>,
    thing: Arc<T>,
}

impl<T> Clone for ThingRef<T> {
    fn clone(&self) -> Self {
        Self {
            handle: Arc::clone(&self.handle),
            thing: Arc::clone(&self.thing),
        }
    }
}

impl<T> fmt::Debug for ThingRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThingRef")
            .field("name", &self.handle.name())
            .finish_non_exhaustive()
    }
}

impl<T: Thing> ThingRef<T> {
    pub(crate) fn new(handle: Arc<ThingHandle>, thing: Arc<T>) -> Self {
        Self { handle, thing }
    }

    /// The name the Thing is served under.
    pub fn name(this: &Self) -> &str {
        this.handle.name()
    }

    /// The Thing itself. Calling its methods directly is a plain Rust call:
    /// no validation and no global lock.
    pub fn thing(this: &Self) -> &Arc<T> {
        &this.thing
    }

    /// The Thing's registry entry.
    pub fn handle(this: &Self) -> &Arc<ThingHandle> {
        &this.handle
    }

    /// Calls an action by name, with any serialisable input, and returns its
    /// output deserialised. See [`ActionEntry::call`].
    pub async fn call_action<O: DeserializeOwned>(
        this: &Self,
        action: &str,
        input: impl Serialize,
    ) -> Result<O, ActionError> {
        let entry = Self::action_entry(this, action)?;
        // `input` and `output` hold their Blobs until they are deserialised.
        let output = entry.call_serialised(serde_json::to_value(&input)?).await?;
        Ok(serde_json::from_value(output.value.clone())?)
    }

    /// Reads a property by name, as a client would.
    pub async fn read_property<V: DeserializeOwned>(
        this: &Self,
        property: &str,
    ) -> Result<V, PropertyError> {
        let entry = this
            .handle
            .property(property)
            .ok_or_else(|| Self::no_property(this, property))?;
        serde_json::from_value(entry.read().await?).map_err(PropertyError::failed)
    }

    /// Writes a property by name, as a client would: the value is
    /// validated, the global lock is taken, and read-only properties refuse.
    pub async fn write_property(
        this: &Self,
        property: &str,
        value: impl Serialize,
    ) -> Result<(), PropertyError> {
        let entry = this
            .handle
            .property(property)
            .ok_or_else(|| Self::no_property(this, property))?;
        entry
            .write(serde_json::to_value(value).map_err(PropertyError::failed)?)
            .await
    }

    /// The typed call behind the generated `{Thing}Actions` methods: the
    /// input is validated as a client's would be (and coerced), then `f`
    /// runs with the global lock and the caller's invocation context.
    #[doc(hidden)]
    pub async fn __call<I, O, F, Fut>(
        this: &Self,
        action: &str,
        input: I,
        f: F,
    ) -> Result<O, ActionError>
    where
        I: Serialize + DeserializeOwned,
        F: FnOnce(Arc<T>, ActionCtx, I) -> Fut,
        Fut: Future<Output = Result<O, ActionError>>,
    {
        let entry = Self::action_entry(this, action)?;
        let validated = entry.validate_input(&serde_json::to_value(&input)?)?;
        let input: I = serde_json::from_value(validated)?;
        let thing = Arc::clone(&this.thing);
        entry.run_in_process(|ctx| f(thing, ctx, input)).await
    }

    fn action_entry<'a>(this: &'a Self, action: &str) -> Result<&'a ActionEntry, ActionError> {
        this.handle.action(action).ok_or_else(|| {
            ActionError::from(anyhow::anyhow!(
                "Thing `{}` has no action named `{action}`",
                this.handle.name()
            ))
        })
    }

    fn no_property(this: &Self, property: &str) -> PropertyError {
        PropertyError::failed(anyhow::anyhow!(
            "Thing `{}` has no property named `{property}`",
            this.handle.name()
        ))
    }
}
