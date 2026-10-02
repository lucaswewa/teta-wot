//! `#[derive(Thing)]`: the struct, its field affordances and its
//! construction from configuration.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Attribute, Data, DeriveInput, Expr, Field, Fields, LitStr, Meta, Type};

use crate::common::{
    Errors, Options, affordance_name, collision_guard, default_expr, describe, docstring, error,
    unwrap_type,
};

/// Field attributes for affordances that later phases implement.
const LATER: [(&str, &str); 2] = [
    ("event", "events arrive with observation"),
    ("stream", "MJPEG streams arrive with Blobs and streams"),
];

/// `#[thing(…)]` on the struct.
#[derive(Default)]
struct ThingOptions {
    title: Option<Expr>,
    description: Option<Expr>,
    config: Option<Type>,
    semantic_types: Vec<Expr>,
    context: Vec<(LitStr, Expr)>,
    interfaces: Vec<syn::Path>,
}

impl ThingOptions {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut options = ThingOptions::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("thing")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("title") {
                    options.title = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("description") {
                    options.description = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("config") {
                    options.config = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("semantic_type") {
                    options.semantic_types.push(meta.value()?.parse()?);
                } else if meta.path.is_ident("context") {
                    meta.parse_nested_meta(|entry| {
                        let prefix = entry
                            .path
                            .get_ident()
                            .ok_or_else(|| entry.error("expected `prefix = \"iri\"`"))?;
                        let prefix = LitStr::new(&prefix.to_string(), prefix.span());
                        options.context.push((prefix, entry.value()?.parse()?));
                        Ok(())
                    })?;
                } else if meta.path.is_ident("interfaces") {
                    meta.parse_nested_meta(|entry| {
                        options.interfaces.push(entry.path.clone());
                        Ok(())
                    })?;
                } else {
                    return Err(meta.error(
                        "unknown option for `#[thing]`; expected one of: title, description, config, semantic_type, context, interfaces",
                    ));
                }
                Ok(())
            })?;
        }
        Ok(options)
    }
}

/// What a field is.
enum Kind {
    /// A data property; `true` for a setting.
    Property(Box<Options>, bool),
    /// A slot, with its default selection.
    Slot(TokenStream),
    Device {
        init: Option<Expr>,
        options: Option<Expr>,
    },
    Plain {
        init: Option<Expr>,
    },
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(error(
            input.generics.span(),
            "a Thing can't be generic: the macros need a concrete type",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(error(ident.span(), "only a struct can derive `Thing`"));
    };
    let fields: Vec<&Field> = match &data.fields {
        Fields::Named(named) => named.named.iter().collect(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return Err(error(
                data.fields.span(),
                "a Thing's fields must be named: they are its affordances",
            ));
        }
    };

    let mut errors = Errors::default();
    let options = errors
        .push_result(ThingOptions::parse(&input.attrs))
        .unwrap_or_default();

    let mut definition = Vec::new();
    let mut guards = Vec::new();
    let mut asserts = Vec::new();
    let mut inits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for field in &fields {
        let Some(kind) = errors.push_result(classify(field)) else {
            continue;
        };
        let name_ident = field.ident.as_ref().expect("named fields");
        let is_affordance = matches!(kind, Kind::Property(..) | Kind::Device { .. });
        if is_affordance {
            let Some(name) = errors.push_result(affordance_name(name_ident)) else {
                continue;
            };
            if !seen.insert(name.clone()) {
                errors.push(error(
                    name_ident.span(),
                    format!("two affordances are named `{name}`"),
                ));
            }
            if matches!(kind, Kind::Property(..)) {
                guards.push(collision_guard(name_ident));
            }
        }
        match kind {
            Kind::Slot(selection) => {
                let is_slot = ["Slot", "OptSlot", "SlotMap"]
                    .iter()
                    .any(|name| unwrap_type(&field.ty, name).is_some());
                if !is_slot {
                    errors.push(error(
                        field.ty.span(),
                        "a `#[slot]` field must be a `Slot<T>`, `OptSlot<T>` or `SlotMap<T>`",
                    ));
                    continue;
                }
                let name = syn::ext::IdentExt::unraw(name_ident).to_string();
                definition.push(quote! {
                    .slot(#name, |thing: &Self| &thing.#name_ident, #selection)
                });
                inits.push(quote!(#name_ident: ::core::default::Default::default()));
            }
            Kind::Property(property, is_setting) => {
                let Some(value) = unwrap_type(&field.ty, "Prop") else {
                    errors.push(error(
                        field.ty.span(),
                        "a `#[property]` or `#[setting]` field must be a `Prop<T>`; for a property computed by a method, put the attribute on the method in `#[thing_impl]`",
                    ));
                    continue;
                };
                let setting = is_setting.then(|| quote!(.setting()));
                if property.blocking {
                    errors.push(error(
                        field.span(),
                        "`blocking` is for property methods, not fields",
                    ));
                }
                asserts.push(
                    quote_spanned!(value.span()=> ::teta_wot::__private::property_value::<#value>();),
                );
                let name = affordance_name(name_ident).unwrap_or_default();
                let readonly = property.readonly.then(|| quote!(.read_only()));
                let described = describe(
                    docstring(&field.attrs).as_deref(),
                    &property.described,
                    true,
                );
                definition.push(quote! {
                    .property(#name, ::teta_wot::DataProperty::new(|thing: &Self| &thing.#name_ident) #readonly #setting #described)
                });
                let default = match &property.default {
                    Some(expr) => {
                        let expr = default_expr(expr);
                        quote!(::teta_wot::Prop::new(#expr))
                    }
                    None => quote_spanned!(field.ty.span()=> ::core::default::Default::default()),
                };
                let constraints = property
                    .constraints()
                    .map(|c| quote!(.with_constraints(#c)));
                inits.push(quote!(#name_ident: #default #constraints));
            }
            Kind::Device {
                init,
                options: device_options,
            } => {
                let name = affordance_name(name_ident).unwrap_or_default();
                definition.push(quote!(.device(#name, |thing: &Self| &thing.#name_ident)));
                let make = match &init {
                    Some(expr) => {
                        quote_spanned!(expr.span()=> ::teta_wot::__private::IntoDriver::into_driver(#expr))
                    }
                    None => {
                        quote_spanned!(field.ty.span()=> ::core::result::Result::Ok(::core::default::Default::default()))
                    }
                };
                let config_type = config_type(&options.config);
                let factory = quote! {{
                    let __wot_config = ::std::sync::Arc::clone(&__wot_config);
                    move || {
                        #[allow(unused_variables)]
                        let config: &#config_type = &__wot_config;
                        #make
                    }
                }};
                let device = match device_options {
                    Some(options) => quote!(::teta_wot::Device::with_options(#factory, #options)),
                    None => quote!(::teta_wot::Device::new(#factory)),
                };
                inits.push(quote!(#name_ident: #device));
            }
            Kind::Plain { init } => {
                let value = match &init {
                    Some(expr) => default_expr(expr),
                    None => quote_spanned!(field.ty.span()=> ::core::default::Default::default()),
                };
                inits.push(quote!(#name_ident: #value));
            }
        }
    }

    let title = match &options.title {
        Some(title) => title.to_token_stream(),
        None => ident.to_string().to_token_stream(),
    };
    let description = match (&options.description, docstring(&input.attrs)) {
        (Some(description), _) => Some(quote!(.description(#description))),
        (None, Some(doc)) => Some(quote!(.description(#doc))),
        (None, None) => None,
    };
    let context = options
        .context
        .iter()
        .map(|(prefix, iri)| quote!(.context_prefix(#prefix, #iri)));
    let types = options
        .semantic_types
        .iter()
        .map(|t| quote!(.semantic_type(#t)));
    let class_name = ident.to_string();
    let interfaces = options.interfaces.iter().map(|path| {
        quote_spanned! {path.span()=>
            .interface::<dyn #path>(|thing| ::std::sync::Arc::new(thing) as ::std::sync::Arc<dyn #path>)
        }
    });

    let config = config_type(&options.config);
    let construct = if matches!(data.fields, Fields::Unit) {
        quote!(Self)
    } else {
        quote!(Self { #(#inits,)* })
    };
    let default_impl = options.config.is_none().then(|| {
        quote! {
            #[automatically_derived]
            impl ::core::default::Default for #ident {
                fn default() -> Self {
                    <Self as ::teta_wot::FromConfig>::from_config(::teta_wot::NoConfig {})
                }
            }
        }
    });
    if !errors.is_empty() {
        // Only the errors, and a stub so that uses of the Thing don't add
        // more: a partial expansion would report fields it skipped.
        let errors = errors.into_tokens();
        return Ok(quote! {
            #errors

            impl ::teta_wot::Thing for #ident {
                fn definition() -> ::teta_wot::ThingDefinition<Self> {
                    ::core::unreachable!()
                }
            }
        });
    }

    Ok(quote! {
        #[automatically_derived]
        impl ::teta_wot::Thing for #ident {
            fn definition() -> ::teta_wot::ThingDefinition<Self> {
                #[allow(unused_imports)]
                use ::teta_wot::__private::{ViaDefault as _, ViaImpl as _};
                #(#asserts)*
                let definition = ::teta_wot::ThingDefinition::new(#title)
                    .class_name(#class_name)
                    #description #(#context)* #(#types)* #(#interfaces)*
                    #(#definition)*;
                (&::teta_wot::__private::Methods::<Self>::new()).definition(definition)
            }

            fn start(
                self: ::std::sync::Arc<Self>,
                ctx: ::teta_wot::ThingCtx,
            ) -> impl ::core::future::Future<Output = ::teta_wot::__private::anyhow::Result<()>> + ::core::marker::Send {
                #[allow(unused_imports)]
                use ::teta_wot::__private::{ViaDefault as _, ViaImpl as _};
                (&::teta_wot::__private::Methods::<Self>::new()).start(self, ctx)
            }

            fn stop(
                self: ::std::sync::Arc<Self>,
                ctx: ::teta_wot::ThingCtx,
            ) -> impl ::core::future::Future<Output = ()> + ::core::marker::Send {
                #[allow(unused_imports)]
                use ::teta_wot::__private::{ViaDefault as _, ViaImpl as _};
                (&::teta_wot::__private::Methods::<Self>::new()).stop(self, ctx)
            }
        }

        #[automatically_derived]
        impl ::teta_wot::FromConfig for #ident {
            type Config = #config;

            fn from_config(config: #config) -> Self {
                #[allow(unused_variables)]
                let __wot_config = ::std::sync::Arc::new(config);
                #[allow(unused_variables)]
                let config: &#config = &__wot_config;
                #construct
            }
        }

        #default_impl

        impl #ident {
            #(#guards)*
        }
    })
}

/// The config type written in `#[thing(config = …)]`, or `NoConfig`.
fn config_type(config: &Option<Type>) -> TokenStream {
    config
        .as_ref()
        .map_or_else(|| quote!(::teta_wot::NoConfig), ToTokens::to_token_stream)
}

/// `#[slot]` (by type), `#[slot(default = "name")]`,
/// `#[slot(default = ["a", "b"])]` or `#[slot(default = None)]`, as a
/// `SlotSelection`.
fn slot_default(attr: &Attribute) -> syn::Result<TokenStream> {
    let by_type = quote!(::teta_wot::SlotSelection::ByType);
    if matches!(attr.meta, Meta::Path(_)) {
        return Ok(by_type);
    }
    let mut selection = by_type;
    attr.parse_nested_meta(|meta| {
        if !meta.path.is_ident("default") {
            return Err(meta.error("unknown option for `#[slot]`; expected: default"));
        }
        let value: Expr = meta.value()?.parse()?;
        let string = |expr: &Expr| match expr {
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => Some(s.value()),
            _ => None,
        };
        selection = match &value {
            Expr::Path(path) if path.path.is_ident("None") => {
                quote!(::teta_wot::SlotSelection::Nothing)
            }
            Expr::Array(array) => {
                let names: Option<Vec<String>> = array.elems.iter().map(string).collect();
                let names = names.ok_or_else(|| {
                    error(array.span(), "slot names must be string literals")
                })?;
                quote!(::teta_wot::SlotSelection::Names(::std::vec![#(::std::string::String::from(#names)),*]))
            }
            other => match string(other) {
                Some(name) => quote!(::teta_wot::SlotSelection::Name(::std::string::String::from(#name))),
                None => {
                    return Err(error(
                        other.span(),
                        "a slot's default is a Thing name (\"stage\"), a list of names ([\"a\", \"b\"]) or None",
                    ));
                }
            },
        };
        Ok(())
    })?;
    Ok(selection)
}

/// Reads a field's attributes.
fn classify(field: &Field) -> syn::Result<Kind> {
    let mut kind: Option<(Kind, &Attribute)> = None;
    let mut init: Option<Expr> = None;
    for attr in &field.attrs {
        let path = attr.path();
        let found = if path.is_ident("property") || path.is_ident("setting") {
            Some(Kind::Property(
                Box::new(Options::parse(
                    attr,
                    &[
                        "readonly",
                        "default",
                        "constraints",
                        "title",
                        "description",
                        "unit",
                        "semantic_type",
                        "global_lock",
                    ],
                )?),
                path.is_ident("setting"),
            ))
        } else if path.is_ident("slot") {
            Some(Kind::Slot(slot_default(attr)?))
        } else if path.is_ident("device") {
            let (mut device_init, mut options) = (None, None);
            if !matches!(attr.meta, Meta::Path(_)) {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("init") {
                        device_init = Some(meta.value()?.parse()?);
                    } else if meta.path.is_ident("options") {
                        options = Some(meta.value()?.parse()?);
                    } else {
                        return Err(meta.error(
                            "unknown option for `#[device]`; expected one of: init, options",
                        ));
                    }
                    Ok(())
                })?;
            }
            Some(Kind::Device {
                init: device_init,
                options,
            })
        } else if path.is_ident("thing") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("init") {
                    init = Some(meta.value()?.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("unknown option for `#[thing]` on a field; expected: init"))
                }
            })?;
            None
        } else if let Some((name, why)) = LATER.iter().find(|(name, _)| path.is_ident(name)) {
            return Err(error(
                path.span(),
                format!("`#[{name}]` isn't supported yet: {why} in a later phase of the plan"),
            ));
        } else {
            None
        };
        if let Some(found) = found {
            if kind.is_some() {
                return Err(error(
                    attr.span(),
                    "a field can be only one kind of affordance",
                ));
            }
            kind = Some((found, attr));
        }
    }
    match kind {
        Some((kind, attr)) => {
            if init.is_some() {
                return Err(error(
                    attr.span(),
                    "`#[thing(init = …)]` is for plain fields; use `default` for a property and `init` in `#[device]`",
                ));
            }
            Ok(kind)
        }
        None => Ok(Kind::Plain { init }),
    }
}
