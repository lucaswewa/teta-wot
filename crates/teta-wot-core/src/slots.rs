//! Thing slots: connections between the Things of one server.
//!
//! A slot is a field of a Thing, filled in by the runtime after every Thing
//! has been built and before settings load and Things start:
//!
//! | Field | Holds | type hint |
//! |---|---|---|
//! | [`Slot<T>`] | exactly one | `T` |
//! | [`OptSlot<T>`] | one or none | `T \| None` |
//! | [`SlotMap<T>`] | any number, by name | `Mapping[str, T]` |
//!
//! `T` is a Thing type (the slot holds a [`ThingRef<T>`], for in-process
//! calls with validation and locking, or an interface, a
//! `dyn Trait` declared with `#[wot::interface]` (the slot holds an
//! `Arc<dyn Trait>`, from any Thing that declares it provides the trait).
//!
//! Which Things fill a slot is decided by the slot's default, which the
//! configuration's `thing_slots` can override: by type (every matching
//! Thing), by name, by names, or none.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::inprocess::ThingRef;
use crate::runtime::ThingHandle;
use crate::thing::Thing;

/// What a slot can hold: a Thing type, or an interface (`dyn Trait`).
///
/// Every Thing type implements it. `#[wot::interface]` implements it for
/// `dyn Trait`.
pub trait SlotTarget: 'static {
    /// What the slot gives access to.
    type Handle: Clone + Send + Sync + 'static;

    /// The handle for a Thing, if the Thing is (or provides) this target.
    fn resolve(thing: &Arc<ThingHandle>) -> Option<Self::Handle>;

    /// The target's name, for error messages.
    fn target_name() -> String;
}

impl<T: Thing> SlotTarget for T {
    type Handle = ThingRef<T>;

    fn resolve(thing: &Arc<ThingHandle>) -> Option<ThingRef<T>> {
        ThingHandle::thing_ref::<T>(thing)
    }

    fn target_name() -> String {
        short_name(std::any::type_name::<T>())
    }
}

/// Resolves an interface on a Thing: for `#[wot::interface]`'s
/// `SlotTarget` implementations.
pub fn resolve_interface<I: ?Sized + 'static>(thing: &Arc<ThingHandle>) -> Option<Arc<I>> {
    ThingHandle::interface::<I>(thing)
}

/// The type's name without its module path, for messages.
pub fn short_name(full: &str) -> String {
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base).to_owned()
}

/// Which Things a slot connects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotSelection {
    /// Every Thing of the slot's type (default, `...`).
    ByType,
    /// The Thing with this name.
    Name(String),
    /// The Things with these names.
    Names(Vec<String>),
    /// No Thing (`None`).
    Nothing,
}

impl fmt::Display for SlotSelection {
    /// prints the target in its error messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotSelection::ByType => f.write_str("..."),
            SlotSelection::Name(name) => f.write_str(name),
            SlotSelection::Names(names) => {
                let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
                write!(f, "[{}]", quoted.join(", "))
            }
            SlotSelection::Nothing => f.write_str("None"),
        }
    }
}

/// How many Things a slot field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Exactly one.
    Single,
    /// One or none.
    Optional,
    /// Any number, by name.
    Map,
}

/// A slot field of a Thing: [`Slot`], [`OptSlot`] or [`SlotMap`].
pub trait SlotField: Send + Sync + 'static {
    /// How many Things it holds.
    const KIND: SlotKind;

    /// The target's name, for error messages.
    fn target_name() -> String;

    /// Whether a Thing can fill the slot.
    fn accepts(thing: &Arc<ThingHandle>) -> bool;

    /// Fills the slot with the picked Things, all of which it accepts.
    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]);
}

/// A slot holding exactly one Thing. It dereferences to the target's handle
/// (a [`ThingRef`], or an `Arc<dyn Trait>`) once connected.
pub struct Slot<T: ?Sized + SlotTarget> {
    value: OnceLock<T::Handle>,
}

impl<T: ?Sized + SlotTarget> Slot<T> {
    /// An unconnected slot.
    pub fn new() -> Self {
        Self {
            value: OnceLock::new(),
        }
    }

    /// The connected Thing, or `None` before the runtime connects the slot.
    pub fn get(&self) -> Option<&T::Handle> {
        self.value.get()
    }
}

impl<T: ?Sized + SlotTarget> Default for Slot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized + SlotTarget> Deref for Slot<T> {
    type Target = T::Handle;

    /// # Panics
    ///
    /// If the slot isn't connected yet: slots are connected when the runtime
    /// is built, so only code that runs before then (such as `from_config`)
    /// can see one unconnected.
    fn deref(&self) -> &T::Handle {
        self.value.get().unwrap_or_else(|| {
            panic!(
                "a slot for {} has not been connected to a Thing yet",
                T::target_name()
            )
        })
    }
}

impl<T: ?Sized + SlotTarget> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot")
            .field("target", &T::target_name())
            .field("connected", &self.value.get().is_some())
            .finish()
    }
}

impl<T: ?Sized + SlotTarget> SlotField for Slot<T> {
    const KIND: SlotKind = SlotKind::Single;

    fn target_name() -> String {
        T::target_name()
    }

    fn accepts(thing: &Arc<ThingHandle>) -> bool {
        T::resolve(thing).is_some()
    }

    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]) {
        if let Some(handle) = picked.first().and_then(|(_, t)| T::resolve(t)) {
            let _ = self.value.set(handle);
        }
    }
}

/// A slot holding one Thing or none.
pub struct OptSlot<T: ?Sized + SlotTarget> {
    value: OnceLock<Option<T::Handle>>,
}

impl<T: ?Sized + SlotTarget> OptSlot<T> {
    /// An unconnected slot.
    pub fn new() -> Self {
        Self {
            value: OnceLock::new(),
        }
    }

    /// The connected Thing, if there is one.
    pub fn get(&self) -> Option<&T::Handle> {
        self.value.get().and_then(Option::as_ref)
    }
}

impl<T: ?Sized + SlotTarget> Default for OptSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized + SlotTarget> fmt::Debug for OptSlot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OptSlot")
            .field("target", &T::target_name())
            .field("connected", &self.get().is_some())
            .finish()
    }
}

impl<T: ?Sized + SlotTarget> SlotField for OptSlot<T> {
    const KIND: SlotKind = SlotKind::Optional;

    fn target_name() -> String {
        T::target_name()
    }

    fn accepts(thing: &Arc<ThingHandle>) -> bool {
        T::resolve(thing).is_some()
    }

    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]) {
        let _ = self
            .value
            .set(picked.first().and_then(|(_, t)| T::resolve(t)));
    }
}

/// A slot holding any number of Things, by name. It dereferences to the
/// map once connected.
pub struct SlotMap<T: ?Sized + SlotTarget> {
    value: OnceLock<IndexMap<String, T::Handle>>,
}

impl<T: ?Sized + SlotTarget> SlotMap<T> {
    /// An unconnected slot.
    pub fn new() -> Self {
        Self {
            value: OnceLock::new(),
        }
    }

    /// The connected Things, or `None` before the runtime connects the slot.
    pub fn get(&self) -> Option<&IndexMap<String, T::Handle>> {
        self.value.get()
    }
}

impl<T: ?Sized + SlotTarget> Default for SlotMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized + SlotTarget> Deref for SlotMap<T> {
    type Target = IndexMap<String, T::Handle>;

    /// # Panics
    ///
    /// If the slot isn't connected yet (see [`Slot`]).
    fn deref(&self) -> &Self::Target {
        self.value
            .get()
            .unwrap_or_else(|| panic!("a slot for {} has not been connected yet", T::target_name()))
    }
}

impl<T: ?Sized + SlotTarget> fmt::Debug for SlotMap<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlotMap")
            .field("target", &T::target_name())
            .field(
                "connected",
                &self.get().map(|m| m.keys().collect::<Vec<_>>()),
            )
            .finish()
    }
}

impl<T: ?Sized + SlotTarget> SlotField for SlotMap<T> {
    const KIND: SlotKind = SlotKind::Map;

    fn target_name() -> String {
        T::target_name()
    }

    fn accepts(thing: &Arc<ThingHandle>) -> bool {
        T::resolve(thing).is_some()
    }

    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]) {
        let map = picked
            .iter()
            .filter_map(|(name, t)| Some((name.clone(), T::resolve(t)?)))
            .collect();
        let _ = self.value.set(map);
    }
}

/// A slot of a running Thing, as the runtime connects it.
pub(crate) trait SlotControl: Send + Sync {
    fn kind(&self) -> SlotKind;
    fn target_name(&self) -> String;
    fn default(&self) -> &SlotSelection;
    fn accepts(&self, thing: &Arc<ThingHandle>) -> bool;
    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]);
}

pub(crate) struct SlotAccess<T, F> {
    pub(crate) thing: Arc<T>,
    pub(crate) accessor: fn(&T) -> &F,
    pub(crate) default: SlotSelection,
}

impl<T: Send + Sync + 'static, F: SlotField> SlotControl for SlotAccess<T, F> {
    fn kind(&self) -> SlotKind {
        F::KIND
    }

    fn target_name(&self) -> String {
        F::target_name()
    }

    fn default(&self) -> &SlotSelection {
        &self.default
    }

    fn accepts(&self, thing: &Arc<ThingHandle>) -> bool {
        F::accepts(thing)
    }

    fn fill(&self, picked: &[(String, Arc<ThingHandle>)]) {
        (self.accessor)(&self.thing).fill(picked);
    }
}

/// Connects one slot, returning the
/// names of the Things it connected to, or an error message.
pub(crate) fn connect(
    host: &str,
    slot_name: &str,
    slot: &dyn SlotControl,
    things: &IndexMap<String, Arc<ThingHandle>>,
    configured: Option<&SlotSelection>,
) -> Result<Vec<String>, String> {
    let used = configured.unwrap_or(slot.default());
    let result = pick(slot, things, used).and_then(|picked| {
        if used == &SlotSelection::Nothing && slot.kind() == SlotKind::Single {
            return Err("it must be set in configuration".to_owned());
        }
        match (slot.kind(), picked.len()) {
            (SlotKind::Map, _) | (_, 1) | (SlotKind::Optional, 0) => Ok(picked),
            (SlotKind::Single, 0) => Err("no matching Thing was found".to_owned()),
            _ => Err("it can't connect to multiple Things".to_owned()),
        }
    });
    match result {
        Ok(picked) => {
            slot.fill(&picked);
            Ok(picked.into_iter().map(|(name, _)| name).collect())
        }
        Err(reason) => {
            let mut message = format!("Can't connect '{host}.{slot_name}' because {reason}. ");
            match configured {
                Some(target) => {
                    message.push_str(&format!("It was configured to connect to '{target}'. "))
                }
                None => message.push_str("It was not configured, and used the default. "),
            }
            match slot.default() {
                SlotSelection::ByType => message.push_str(&format!(
                    "The default searches for Things by type: '{}'.",
                    slot.target_name()
                )),
                default => message.push_str(&format!("The default is '{default}'.")),
            }
            Err(message)
        }
    }
}

fn pick(
    slot: &dyn SlotControl,
    things: &IndexMap<String, Arc<ThingHandle>>,
    selection: &SlotSelection,
) -> Result<Vec<(String, Arc<ThingHandle>)>, String> {
    let named = |name: &String| -> Result<(String, Arc<ThingHandle>), String> {
        let thing = things
            .get(name)
            .ok_or_else(|| format!("{name} is not the name of a Thing"))?;
        if !slot.accepts(thing) {
            return Err(format!("{name} is the wrong type"));
        }
        Ok((name.clone(), Arc::clone(thing)))
    };
    match selection {
        SlotSelection::Nothing => Ok(Vec::new()),
        SlotSelection::ByType => Ok(things
            .iter()
            .filter(|(_, t)| slot.accepts(t))
            .map(|(n, t)| (n.clone(), Arc::clone(t)))
            .collect()),
        SlotSelection::Name(name) => Ok(vec![named(name)?]),
        SlotSelection::Names(names) => names.iter().map(named).collect(),
    }
}

/// The start order: dependencies (the Things a Thing's slots connect to)
/// before the Things that use them, otherwise in configuration order. With
/// a dependency cycle, the configuration order is used as it is.
pub(crate) fn start_order(
    names: &[String],
    dependencies: &IndexMap<String, Vec<String>>,
) -> Result<Vec<String>, Vec<String>> {
    let mut order: Vec<String> = Vec::with_capacity(names.len());
    let mut remaining: Vec<&String> = names.iter().collect();
    while !remaining.is_empty() {
        let ready = remaining.iter().position(|name| {
            dependencies
                .get(*name)
                .into_iter()
                .flatten()
                .all(|dep| dep == *name || order.contains(dep) || !names.contains(dep))
        });
        match ready {
            Some(index) => order.push(remaining.remove(index).clone()),
            None => return Err(remaining.into_iter().cloned().collect()),
        }
    }
    Ok(order)
}
