//! Proc-macros for authoring `wot-rs` Things: `#[derive(Thing)]` and
//! `#[thing_impl]`. Use them through the `teta_wot` crate (`teta_wot::Thing`,
//! `teta_wot::thing_impl`), whose documentation they refer to.
//!
//! Both expand to the builder API of `teta-wot-core`.

use proc_macro::TokenStream;

mod common;
mod derive;
mod interface;
mod thing_impl;

/// Makes a trait an interface that slots can target (`Slot<dyn Trait>`,
/// `OptSlot<dyn Trait>`, `SlotMap<dyn Trait>`), filled by any Thing that
/// lists it in `#[thing(interfaces(Trait))]`.
///
/// The trait needs `Send + Sync` supertraits and must be dyn-compatible:
/// write async methods as returning a boxed future. A Thing provides it by
/// implementing it for `ThingRef<Self>`, so calls through the interface can
/// use the in-process action wrappers, with their validation and locking.
///
/// ```ignore
/// #[wot::interface]
/// pub trait CameraApi: Send + Sync {
///     fn sharpness(&self) -> wot::BoxFuture<'_, Result<f64, ActionError>>;
/// }
///
/// #[derive(Thing)]
/// #[thing(interfaces(CameraApi))]
/// pub struct SimCamera { /* … */ }
///
/// impl CameraApi for ThingRef<SimCamera> {
///     fn sharpness(&self) -> wot::BoxFuture<'_, Result<f64, ActionError>> {
///         self.measure_sharpness() // a generated `SimCameraActions` method
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn interface(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = proc_macro2::TokenStream::from(args);
    let item = syn::parse_macro_input!(input as syn::ItemTrait);
    interface::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Makes a struct a Thing: its fields become properties, events and devices, and it
/// can be built from a typed configuration.
///
/// ```ignore
/// use wot::prelude::*;
///
/// /// A computer-controlled light, our first example Thing.
/// #[derive(Thing)]
/// pub struct Light {
///     /// The brightness of the light, in % of maximum.
///     #[property(default = 100, ge = 0, le = 100, unit = "percent")]
///     brightness: Prop<u8>,
///
///     /// Whether the light is currently on.
///     #[property(default = false, readonly)]
///     is_on: Prop<bool>,
/// }
/// ```
///
/// **The struct.** Its doc comment is the Thing's description; its title is
/// the struct's name. `#[thing(…)]` options:
///
/// - `title = "…"`, `description = "…"`: override them;
/// - `config = Type`: the typed configuration, deserialised from the
///   Thing's `kwargs`, and available as `config` in `default` and `init`
///   expressions. Without it, the Thing takes no configuration and also
///   implements `Default`;
/// - `semantic_type = "…"` (repeatable): the TD's `@type`;
/// - `context(prefix = "iri", …)`: prefixes for the TD's `@context`;
/// - `interfaces(Trait, …)`: the `#[wot::interface]` traits the Thing
///   provides to slots, each implemented for `ThingRef<Self>`.
///
/// **Fields.**
///
/// - `#[property(…)]` on a `Prop<T>` field: a data property. Its doc
///   comment gives the title (first line) and description.
///   Options: `readonly`; `default = expr`; the constraints `ge`, `gt`, `le`,
///   `lt`, `multiple_of`, `min_length`, `max_length`, `pattern` and
///   `allow_inf_nan`; `title`, `description`, `unit`, `semantic_type`
///   (repeatable); `global_lock = false`.
/// - `#[device(init = expr, options = expr)]` on a `Device<D>` field: a
///   device, opened before the Thing starts. `init` makes the driver (a `D`
///   or a `Result<D, _>`) on the device's thread; without it, `D::default()`.
/// - Other fields are initialised with `#[thing(init = expr)]`, or
///   `Default::default()`.
/// - `#[setting(…)]` on a `Prop<T>` field: a data property saved to the
///   Thing's settings file after every change and loaded when the server
///   starts. Options are those of `#[property]`.
/// - `#[slot]` on a `Slot<T>`, `OptSlot<T>` or `SlotMap<T>` field: filled
///   with other Things when the server is built. `T` is a Thing
///   type or a `dyn` interface. By default it connects by type;
///   `default = "name"`, `default = ["a", "b"]` or `default = None` change
///   that, and the configuration's `thing_slots` overrides it.
/// - `#[event]` on an `Event<T>` field: an event, which the Thing emits
///   with `self.field.emit(data)`, from async or synchronous code. Its doc
///   comment gives the title and description, and `T`'s
///   schema the TD's `data`. Options: `title`, `description`,
///   `semantic_type` (repeatable).
/// - `#[stream]` or `#[stream(buffer = n)]` on an `MjpegStream` field: an
///   MJPEG stream at `/{thing}/{name}`, with a viewer page at
///   `…/viewer`, linked from the TD. Frames are added with
///   `self.field.add_frame(jpeg)`, from any thread. The ring buffer keeps
///   `n` frames (10 by default).
///
/// Methods (actions, functional properties, endpoints, lifecycle hooks) go in
/// a `#[thing_impl]` block.
#[proc_macro_derive(
    Thing,
    attributes(thing, property, device, setting, event, slot, stream)
)]
pub fn derive_thing(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Makes methods of a Thing its actions, functional properties, custom
/// endpoints and lifecycle hooks.
///
/// ```ignore
/// #[thing_impl]
/// impl Light {
///     /// Swap the light between on and off.
///     #[action]
///     async fn toggle(&self) -> Result<bool, ActionError> {
///         self.is_on.update(|on| *on = !*on)?;
///         Ok(self.is_on.get())
///     }
///
///     /// A human-readable status of the light.
///     #[property]
///     async fn status(&self) -> String {
///         if self.is_on.get() { format!("On at {}%.", self.brightness.get()) } else { "Off.".into() }
///     }
/// }
/// ```
///
/// Every method takes `&self`, and must be `async` unless marked `blocking`
/// (then it runs on a blocking thread). Doc comments give the title and
/// description.
///
/// - `#[action(…)]`: an action. Parameters become the input's fields;
///   `#[param(default)]`, `#[param(default = expr)]` and
///   `#[param(description = "…")]` describe them. Parameters of these types
///   are supplied by the runtime rather than the input: `ActionCtx` (the
///   invocation context), `Server` (the server) and `Dep<S>` (a registered
///   service). `#[input]` on the only other parameter makes its
///   type the whole input. It may return a value, a
///   `Result<T, E>` (with `E: Into<ActionError>`), or nothing. Options:
///   `blocking`, `retention = seconds`, `global_lock = false`, `title`,
///   `description`, `semantic_type`, `synchronous` (the caller waits for
///   the output, in the `wot` wire profile).
/// - `#[property(…)]`: a functional property's getter, taking only `&self`
///   and returning `T` or `Result<T, E>` (with `E: Into<PropertyError>`).
///   `#[setter(name)]` and `#[resetter(name)]` (optionally with `blocking`)
///   add a setter and a resetter to the property `name`. Options are those
///   of a data property, plus `blocking`. `#[setting(…)]` in place of
///   `#[property(…)]` makes it a setting, saved after each write through the
///   property.
/// - `#[endpoint(get, "path")]`: a custom HTTP endpoint at
///   `/{thing}/{path}` (the path defaults to the method's name). Its
///   parameters are axum extractors, and it returns a type that implements
///   `IntoResponse`. Return a concrete type (such as `Response`) rather than
///   `impl IntoResponse`, which in edition 2024 would borrow `&self`.
///   `rel = "related"` (and optionally `media_type = "text/csv"`) lists it
///   in the TD's `links`.
/// - `#[on_start]`, `#[on_stop]`: lifecycle hooks, taking `&self` and
///   optionally a `ThingCtx`.
/// - `#[thing_state]`: a synchronous `&self` method returning a
///   serialisable summary of the Thing, for `Server::thing_states`.
///
/// It also generates the trait `{Thing}Actions`, implemented for
/// `ThingRef<{Thing}>`, with one method per action for in-process calls.
#[proc_macro_attribute]
pub fn thing_impl(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = proc_macro2::TokenStream::from(args);
    let item = syn::parse_macro_input!(input as syn::ItemImpl);
    thing_impl::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
