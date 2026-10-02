//! The server as a Thing sees it:
//! the other Things, shared services, the application configuration and the
//! settings folder.
//!
//! A [`Server`] is cheap to clone. Actions get it from
//! [`ActionCtx::server`](crate::ActionCtx::server), lifecycle hooks from
//! [`ThingCtx::server`](crate::ThingCtx::server), and an action parameter of
//! type `Server` or [`Dep<T>`] receives it (or one service) directly.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use indexmap::IndexMap;
use serde_json::Value;

use crate::inprocess::ThingRef;
use crate::runtime::ThingHandle;
use crate::thing::Thing;

/// A service: an `Arc<T>` (for any `T`, including `dyn Trait`), stored as
/// `Any`, with its type's name for error messages.
pub(crate) struct Service {
    pub(crate) value: Box<dyn Any + Send + Sync>,
    pub(crate) type_name: &'static str,
}

/// What the Things of one runtime share.
#[derive(Default)]
pub(crate) struct ServerShared {
    /// The Things, filled in once they are all built. Weak, so that the
    /// runtime alone keeps them alive.
    pub(crate) things: OnceLock<IndexMap<String, Weak<ThingHandle>>>,
    pub(crate) services: HashMap<TypeId, Service>,
    pub(crate) application_config: Option<Value>,
    pub(crate) settings_folder: Option<PathBuf>,
}

/// A handle to the server that runs a Thing.
#[derive(Clone)]
pub struct Server {
    pub(crate) shared: Arc<ServerShared>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let services: Vec<&str> = self.shared.services.values().map(|s| s.type_name).collect();
        f.debug_struct("Server")
            .field("things", &self.thing_names())
            .field("services", &services)
            .finish_non_exhaustive()
    }
}

impl Server {
    pub(crate) fn new(shared: ServerShared) -> Self {
        Self {
            shared: Arc::new(shared),
        }
    }

    /// A server with no Things, services or configuration, for running Thing
    /// code in tests.
    pub fn detached() -> Self {
        Self::new(ServerShared::default())
    }

    fn handles(&self) -> impl Iterator<Item = (&str, Arc<ThingHandle>)> {
        self.shared
            .things
            .get()
            .into_iter()
            .flat_map(|things| things.iter())
            .filter_map(|(name, handle)| Some((name.as_str(), handle.upgrade()?)))
    }

    /// The names of the Things on the server, in configuration order.
    pub fn thing_names(&self) -> Vec<String> {
        self.handles().map(|(name, _)| name.to_owned()).collect()
    }

    /// The Thing named `name`, if it is a `T`.
    pub fn thing_ref<T: Thing>(&self, name: &str) -> Option<ThingRef<T>> {
        let handle = self.shared.things.get()?.get(name)?.upgrade()?;
        ThingHandle::thing_ref(&handle)
    }

    /// Every Thing that is a `T` (`things_by_class`).
    pub fn things_of_type<T: Thing>(&self) -> Vec<ThingRef<T>> {
        self.handles()
            .filter_map(|(_, handle)| ThingHandle::thing_ref(&handle))
            .collect()
    }

    /// A shared service registered with the server builder.
    pub fn service<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.shared
            .services
            .get(&TypeId::of::<T>())
            .and_then(|s| s.value.downcast_ref::<Arc<T>>())
            .cloned()
    }

    /// A copy of the `application_config` from the configuration file.
    pub fn application_config(&self) -> Option<Value> {
        self.shared.application_config.clone()
    }

    /// The state of every Thing, by name (`get_thing_states`): what
    /// each Thing's `#[thing_state]` method returns, or `{}`.
    pub fn thing_states(&self) -> IndexMap<String, Value> {
        self.handles()
            .map(|(name, handle)| (name.to_owned(), handle.thing_state()))
            .collect()
    }

    /// The server's settings folder, if settings are persisted.
    pub fn settings_folder(&self) -> Option<&Path> {
        self.shared.settings_folder.as_deref()
    }
}

/// A shared service, injected into an action parameter of type `Dep<T>`.
/// Services are registered once, with the server builder, and
/// live as long as the server.
pub struct Dep<T: ?Sized>(Arc<T>);

impl<T: ?Sized> Dep<T> {
    /// Wraps a service.
    pub fn new(service: Arc<T>) -> Self {
        Self(service)
    }

    /// The shared service.
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T: ?Sized> Clone for Dep<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> Deref for Dep<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Dep<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Dep").field(&&*self.0).finish()
    }
}
