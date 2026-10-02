//! The Thing types a server can build from a configuration file.
//!

use std::fmt;
use std::sync::Arc;

use serde_json::{Map, Value};
use teta_wot_core::FromConfig;

use crate::ThingServerBuilder;

type AddThing = Arc<
    dyn Fn(ThingServerBuilder, &str, Vec<Value>, Map<String, Value>) -> ThingServerBuilder
        + Send
        + Sync,
>;

struct Entry {
    names: Vec<String>,
    type_name: &'static str,
    add: AddThing,
}

/// The Thing types a server can build from configuration, by name.
#[derive(Default, Clone)]
pub struct ThingRegistry {
    entries: Vec<Arc<Entry>>,
}

impl fmt::Debug for ThingRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|e| (&e.names, e.type_name)))
            .finish()
    }
}

/// `a.b:C`, `a.b.C` and `a::b::C` are the same name.
pub fn normalise(name: &str) -> String {
    name.trim().replace("::", ".").replace(':', ".")
}

impl ThingRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `T` under `name`, and under its short Rust type name.
    #[must_use]
    pub fn register<T: FromConfig>(self, name: &str) -> Self {
        self.register_as::<T>(&[name])
    }

    /// Registers `T` under several names, and under
    /// its short Rust type name.
    #[must_use]
    pub fn register_as<T: FromConfig>(mut self, names: &[&str]) -> Self {
        let type_name = std::any::type_name::<T>();
        let mut all: Vec<String> = names.iter().map(|n| normalise(n)).collect();
        let short = teta_wot_core::slots::short_name(type_name);
        if !all.contains(&short) {
            all.push(short);
        }
        let add: AddThing = Arc::new(|builder, thing, args, kwargs| {
            builder.thing_from_args::<T>(thing, args, kwargs)
        });
        self.entries.push(Arc::new(Entry {
            names: all,
            type_name,
            add,
        }));
        self
    }

    /// Whether a class name (import string) is registered.
    pub fn contains(&self, cls: &str) -> bool {
        self.find(cls).is_some()
    }

    /// Every registered name.
    pub fn names(&self) -> Vec<String> {
        self.entries.iter().flat_map(|e| e.names.clone()).collect()
    }

    fn find(&self, cls: &str) -> Option<&Arc<Entry>> {
        let wanted = normalise(cls);
        // The last registration of a name wins, so applications can
        // override a library's.
        self.entries
            .iter()
            .rev()
            .find(|e| e.names.contains(&wanted))
    }

    /// Adds the Thing named `thing` of class `cls` to a server builder.
    pub(crate) fn add(
        &self,
        builder: ThingServerBuilder,
        thing: &str,
        cls: &str,
        args: Vec<Value>,
        kwargs: Map<String, Value>,
    ) -> Option<ThingServerBuilder> {
        let entry = self.find(cls)?;
        Some((entry.add)(builder, thing, args, kwargs))
    }
}
