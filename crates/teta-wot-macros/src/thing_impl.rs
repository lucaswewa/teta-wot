//! `#[thing_impl]`: method affordances, endpoints, lifecycle hooks and the
//! typed in-process wrappers.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::parse::ParseStream;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, Pat, Token, Type,
};

use crate::common::{
    Errors, Options, Output, affordance_name, borrows, collision_guard, default_expr, describe,
    docstring, error, is_named,
};

/// The attributes that give a method a role.
const ROLES: [&str; 9] = [
    "action",
    "property",
    "setting",
    "setter",
    "resetter",
    "endpoint",
    "on_start",
    "on_stop",
    "thing_state",
];

/// A parameter of an action.
enum Param {
    /// `ActionCtx`, supplied by the runtime.
    Ctx,
    /// A field of the generated input model.
    Field {
        ident: Ident,
        ty: Type,
        default: Option<Option<Expr>>,
        description: Option<LitStr>,
    },
    /// `#[input]`: the whole input.
    Whole { ident: Ident, ty: Type },
    /// `Dep<S>` (with `S`) or `Server` (without): injected from the server.
    Injected(Option<Type>),
}

struct Action {
    method: ImplItemFn,
    name: String,
    options: Options,
    params: Vec<Param>,
}

struct Accessor {
    method: ImplItemFn,
    blocking: bool,
}

struct Property {
    method: ImplItemFn,
    name: String,
    options: Options,
    setter: Option<Accessor>,
    resetter: Option<Accessor>,
    setting: bool,
}

struct Endpoint {
    method: ImplItemFn,
    http_method: String,
    path: String,
}

pub fn expand(args: TokenStream, mut item: ItemImpl) -> syn::Result<TokenStream> {
    if !args.is_empty() {
        return Err(error(args.span(), "`#[thing_impl]` takes no arguments"));
    }
    if let Some((_, path, _)) = &item.trait_ {
        return Err(error(
            path.span(),
            "`#[thing_impl]` goes on the Thing's own `impl` block (`impl MyThing { … }`), not on a trait impl",
        ));
    }
    if !item.generics.params.is_empty() {
        return Err(error(item.generics.span(), "a Thing can't be generic"));
    }
    let self_ty = (*item.self_ty).clone();
    let Some(type_ident) = crate::common::last_segment(&self_ty).map(|s| s.ident.clone()) else {
        return Err(error(self_ty.span(), "`#[thing_impl]` needs a named type"));
    };

    let mut errors = Errors::default();
    let mut actions: Vec<Action> = Vec::new();
    let mut properties: Vec<Property> = Vec::new();
    let mut endpoints: Vec<Endpoint> = Vec::new();
    let mut on_start: Option<ImplItemFn> = None;
    let mut on_stop: Option<ImplItemFn> = None;
    let mut thing_state: Option<ImplItemFn> = None;
    let mut accessors: Vec<(Ident, bool, Accessor)> = Vec::new();
    let mut names: HashMap<String, proc_macro2::Span> = HashMap::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let roles: Vec<Attribute> = method
            .attrs
            .iter()
            .filter(|a| ROLES.iter().any(|r| a.path().is_ident(r)))
            .cloned()
            .collect();
        let original = method.clone();
        crate::common::strip_attrs(&mut method.attrs, &ROLES);
        for input in &mut method.sig.inputs {
            if let FnArg::Typed(typed) = input {
                crate::common::strip_attrs(&mut typed.attrs, &["param", "input"]);
            }
        }
        let Some(role) = roles.first() else {
            if let Some(param) = original.sig.inputs.iter().find_map(|i| match i {
                FnArg::Typed(t) => t
                    .attrs
                    .iter()
                    .find(|a| a.path().is_ident("param") || a.path().is_ident("input")),
                FnArg::Receiver(_) => None,
            }) {
                errors.push(error(
                    param.span(),
                    "`#[param]` and `#[input]` are for action parameters",
                ));
            }
            continue;
        };
        if roles.len() > 1 {
            errors.push(error(roles[1].span(), "a method can have only one role"));
            continue;
        }
        // The method as the macros see it: with its parameter attributes.
        let mut seen = original;
        crate::common::strip_attrs(&mut seen.attrs, &ROLES);
        let role_name = role
            .path()
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        let result = match role_name.as_str() {
            "action" => parse_action(role, seen).map(|a| {
                check_name(&mut names, &mut errors, &a.name, &a.method.sig.ident);
                actions.push(a);
            }),
            "property" | "setting" => parse_property(role, seen).map(|mut p| {
                p.setting = role_name == "setting";
                check_name(&mut names, &mut errors, &p.name, &p.method.sig.ident);
                properties.push(p);
            }),
            "thing_state" => check_state(&seen).map(|()| {
                if thing_state.is_some() {
                    errors.push(error(
                        role.span(),
                        "only one method can be `#[thing_state]`",
                    ));
                }
                thing_state = Some(seen);
            }),
            "setter" | "resetter" => parse_accessor(role, seen).map(|(target, accessor)| {
                accessors.push((target, role_name == "setter", accessor));
            }),
            "endpoint" => parse_endpoint(role, seen).map(|e| endpoints.push(e)),
            "on_start" | "on_stop" => check_hook(&role_name, &seen).map(|()| {
                let slot = if role_name == "on_start" {
                    &mut on_start
                } else {
                    &mut on_stop
                };
                if slot.is_some() {
                    errors.push(error(
                        role.span(),
                        format!("only one method can be `#[{role_name}]`"),
                    ));
                }
                *slot = Some(seen);
            }),
            _ => Ok(()),
        };
        errors.push_result(result);
    }

    for (target, is_setter, accessor) in accessors {
        let kind = if is_setter { "setter" } else { "resetter" };
        let Some(property) = properties.iter_mut().find(|p| p.method.sig.ident == target) else {
            errors.push(error(
                target.span(),
                format!("no `#[property]` method named `{target}` in this block for this {kind}"),
            ));
            continue;
        };
        let slot = if is_setter {
            &mut property.setter
        } else {
            &mut property.resetter
        };
        if slot.is_some() {
            errors.push(error(
                target.span(),
                format!("`{target}` already has a {kind}"),
            ));
        }
        *slot = Some(accessor);
    }

    let mut paths: HashMap<(String, String), ()> = HashMap::new();
    for endpoint in &endpoints {
        if paths
            .insert((endpoint.http_method.clone(), endpoint.path.clone()), ())
            .is_some()
        {
            errors.push(error(
                endpoint.method.sig.ident.span(),
                format!(
                    "two endpoints answer {} {}",
                    endpoint.http_method, endpoint.path
                ),
            ));
        }
    }

    if !errors.is_empty() {
        let errors = errors.into_tokens();
        return Ok(quote!(#item #errors));
    }

    let mut asserts = Vec::new();
    let mut definition = Vec::new();
    let mut inputs = Vec::new();
    let mut guards = Vec::new();
    let mut wrappers = Vec::new();
    for property in &properties {
        guards.push(collision_guard(&property.method.sig.ident));
        definition.push(property_tokens(property, &mut asserts));
    }
    for action in &actions {
        guards.push(collision_guard(&action.method.sig.ident));
        let generated = action_tokens(action, &type_ident, &self_ty, &mut asserts);
        definition.push(generated.definition);
        inputs.push(generated.input);
        wrappers.push(generated.wrapper);
    }
    for endpoint in &endpoints {
        definition.push(endpoint_tokens(endpoint));
    }
    if let Some(state) = &thing_state {
        let method = &state.sig.ident;
        definition.push(quote! {
            .thing_state(|__thing: &Self| {
                ::teta_wot::__private::serde_json::to_value(__thing.#method()).unwrap_or_default()
            })
        });
    }
    let start = on_start.map(|m| hook_tokens(&m, true));
    let stop = on_stop.map(|m| hook_tokens(&m, false));
    let actions_trait = actions_trait(&type_ident, &self_ty, &wrappers);

    Ok(quote! {
        #item

        #(#inputs)*

        #[automatically_derived]
        impl ::teta_wot::__private::ThingMethods for #self_ty {
            fn methods(definition: ::teta_wot::ThingDefinition<Self>) -> ::teta_wot::ThingDefinition<Self> {
                #(#asserts)*
                definition #(#definition)*
            }

            #start
            #stop
        }

        #actions_trait

        impl #self_ty {
            #(#guards)*
        }
    })
}

fn check_name(
    names: &mut HashMap<String, proc_macro2::Span>,
    errors: &mut Errors,
    name: &str,
    ident: &Ident,
) {
    if names.insert(name.to_owned(), ident.span()).is_some() {
        errors.push(error(
            ident.span(),
            format!("two affordances are named `{name}`"),
        ));
    }
}

/// The method must take `&self`, and be `async` unless `blocking`. `what`
/// names it without an article ("action", "property getter").
fn check_method(method: &ImplItemFn, what: &str, blocking: Option<bool>) -> syn::Result<()> {
    let article = if what.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    match method.sig.inputs.first() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
        other => {
            let span = other.map_or_else(|| method.sig.ident.span(), Spanned::span);
            return Err(error(
                span,
                format!(
                    "{article} {what} must take `&self`: a Thing is shared, and its affordances run concurrently"
                ),
            ));
        }
    }
    let is_async = method.sig.asyncness.is_some();
    match blocking {
        Some(true) if is_async => Err(error(
            method.sig.asyncness.span(),
            format!("a `blocking` {what} must not be `async`"),
        )),
        Some(false) if !is_async => Err(error(
            method.sig.ident.span(),
            format!(
                "this {what} isn't `async`, so it would block the server's async threads: make it `async`, or mark it `blocking` to run it on a blocking thread"
            ),
        )),
        None if !is_async => Err(error(
            method.sig.ident.span(),
            format!("{article} {what} must be `async`"),
        )),
        _ => Ok(()),
    }
}

fn typed_params(method: &ImplItemFn) -> impl Iterator<Item = &syn::PatType> {
    method.sig.inputs.iter().filter_map(|i| match i {
        FnArg::Typed(t) => Some(t),
        FnArg::Receiver(_) => None,
    })
}

fn param_ident(pat: &Pat) -> syn::Result<Ident> {
    match pat {
        Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => Ok(p.ident.clone()),
        _ => Err(error(
            pat.span(),
            "an action parameter must be a plain name: its name is the input's field name",
        )),
    }
}

fn parse_action(attr: &Attribute, method: ImplItemFn) -> syn::Result<Action> {
    let options = Options::parse(
        attr,
        &[
            "blocking",
            "retention",
            "title",
            "description",
            "semantic_type",
            "global_lock",
        ],
    )?;
    check_method(&method, "action", Some(options.blocking))?;
    let name = affordance_name(&method.sig.ident)?;
    let mut params = Vec::new();
    let mut errors = Errors::default();
    for typed in typed_params(&method) {
        let ident = match param_ident(&typed.pat) {
            Ok(ident) => ident,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let ty = (*typed.ty).clone();
        if is_named(&ty, "ActionCtx") {
            params.push(Param::Ctx);
            continue;
        }
        if let Some(service) = crate::common::unwrap_type(&ty, "Dep") {
            params.push(Param::Injected(Some(service.clone())));
            continue;
        }
        if is_named(&ty, "Server") {
            params.push(Param::Injected(None));
            continue;
        }
        if borrows(&ty) {
            errors.push(error(ty.span(), "an action parameter must own its data (for example `String`, not `&str`): it is deserialised from the request"));
            continue;
        }
        let mut default = None;
        let mut description = None;
        let mut whole = false;
        for attr in &typed.attrs {
            if attr.path().is_ident("input") {
                whole = true;
            } else if attr.path().is_ident("param") {
                errors.push_result(attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("default") {
                        default = Some(if meta.input.peek(Token![=]) {
                            Some(meta.value()?.parse()?)
                        } else {
                            None
                        });
                    } else if meta.path.is_ident("description") {
                        description = Some(meta.value()?.parse()?);
                    } else {
                        return Err(meta.error(
                            "unknown option for `#[param]`; expected one of: default, description",
                        ));
                    }
                    Ok(())
                }));
            }
        }
        params.push(if whole {
            Param::Whole { ident, ty }
        } else {
            Param::Field {
                ident,
                ty,
                default,
                description,
            }
        });
    }
    let data_params = params
        .iter()
        .filter(|p| !matches!(p, Param::Ctx | Param::Injected(_)))
        .count();
    if params.iter().any(|p| matches!(p, Param::Whole { .. })) && data_params > 1 {
        errors.push(error(
            method.sig.inputs.span(),
            "an `#[input]` parameter must be the action's only parameter besides `ActionCtx`",
        ));
    }
    if params.iter().filter(|p| matches!(p, Param::Ctx)).count() > 1 {
        errors.push(error(
            method.sig.inputs.span(),
            "an action takes at most one `ActionCtx`",
        ));
    }
    errors.finish()?;
    Ok(Action {
        method,
        name,
        options,
        params,
    })
}

fn parse_property(attr: &Attribute, method: ImplItemFn) -> syn::Result<Property> {
    let options = Options::parse(
        attr,
        &[
            "blocking",
            "readonly",
            "default",
            "constraints",
            "title",
            "description",
            "unit",
            "semantic_type",
            "global_lock",
        ],
    )?;
    check_method(&method, "property getter", Some(options.blocking))?;
    if let Some(extra) = typed_params(&method).next() {
        return Err(error(extra.span(), "a property getter takes only `&self`"));
    }
    let name = affordance_name(&method.sig.ident)?;
    Ok(Property {
        method,
        name,
        options,
        setter: None,
        resetter: None,
        setting: false,
    })
}

/// A `#[thing_state]` method takes only `&self`, isn't async, and returns a
/// serialisable value.
fn check_state(method: &ImplItemFn) -> syn::Result<()> {
    check_method(method, "`#[thing_state]` method", Some(true)).map_err(|e| {
        if method.sig.asyncness.is_some() {
            error(
                method.sig.asyncness.span(),
                "a `#[thing_state]` method can't be `async`: it is read often, and should be quick",
            )
        } else {
            e
        }
    })?;
    if let Some(extra) = typed_params(method).next() {
        return Err(error(
            extra.span(),
            "a `#[thing_state]` method takes only `&self`",
        ));
    }
    Ok(())
}

fn parse_accessor(attr: &Attribute, method: ImplItemFn) -> syn::Result<(Ident, Accessor)> {
    let kind = attr
        .path()
        .get_ident()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut target: Option<Ident> = None;
    let mut blocking = false;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("blocking") {
            blocking = true;
        } else if let Some(ident) = meta.path.get_ident() {
            if target.is_some() {
                return Err(meta.error("name one property"));
            }
            target = Some(ident.clone());
        } else {
            return Err(meta.error("expected the property's name"));
        }
        Ok(())
    })?;
    let target = target.ok_or_else(|| {
        error(
            attr.span(),
            format!("`#[{kind}(name)]` needs the name of the property's getter"),
        )
    })?;
    check_method(&method, &format!("property {kind}"), Some(blocking))?;
    let expected = usize::from(kind == "setter");
    if typed_params(&method).count() != expected {
        return Err(error(
            method.sig.inputs.span(),
            if expected == 1 {
                "a setter takes `&self` and the new value"
            } else {
                "a resetter takes only `&self`"
            },
        ));
    }
    Ok((target, Accessor { method, blocking }))
}

fn parse_endpoint(attr: &Attribute, method: ImplItemFn) -> syn::Result<Endpoint> {
    let (verb, path) = attr.parse_args_with(|input: ParseStream<'_>| {
        let verb: Ident = input.parse()?;
        let path = if input.parse::<Option<Token![,]>>()?.is_some() {
            Some(input.parse::<LitStr>()?)
        } else {
            None
        };
        Ok((verb, path))
    })?;
    let http_method = verb.to_string().to_ascii_uppercase();
    if !["GET", "POST", "PUT", "DELETE", "PATCH"].contains(&http_method.as_str()) {
        return Err(error(
            verb.span(),
            "expected an HTTP method: get, post, put, delete or patch",
        ));
    }
    check_method(&method, "endpoint", None)?;
    let path = path.map_or_else(|| method.sig.ident.to_string(), |p| p.value());
    Ok(Endpoint {
        method,
        http_method,
        path,
    })
}

fn check_hook(role: &str, method: &ImplItemFn) -> syn::Result<()> {
    check_method(method, &format!("`#[{role}]` hook"), None)?;
    for typed in typed_params(method) {
        if !is_named(&typed.ty, "ThingCtx") {
            return Err(error(
                typed.span(),
                "a lifecycle hook takes `&self` and, optionally, a `ThingCtx`",
            ));
        }
    }
    if role == "on_stop" && !matches!(Output::of(&method.sig.output), Output::Unit) {
        return Err(error(
            method.sig.output.span(),
            "an `#[on_stop]` hook returns nothing: handle errors inside it",
        ));
    }
    Ok(())
}

fn property_tokens(property: &Property, asserts: &mut Vec<TokenStream>) -> TokenStream {
    let error_ty = quote!(::teta_wot::PropertyError);
    let getter = &property.method.sig.ident;
    let output = Output::of(&property.method.sig.output);
    let value = output.value_type();
    asserts.push(quote_spanned!(value.span()=> ::teta_wot::__private::property_value::<#value>();));
    let name = &property.name;
    let make = if property.options.blocking {
        let call = output.wrap(quote!(__thing.#getter()), &error_ty);
        quote!(::teta_wot::FunctionalProperty::blocking_getter(|__thing: &Self| #call))
    } else {
        let call = output.wrap(quote!(__thing.#getter().await), &error_ty);
        quote!(::teta_wot::FunctionalProperty::getter(|__thing: ::std::sync::Arc<Self>| async move { #call }))
    };
    let setter = property.setter.as_ref().map(|accessor| {
        let method = &accessor.method.sig.ident;
        let out = Output::of(&accessor.method.sig.output);
        if accessor.blocking {
            let call = out.wrap(quote!(__thing.#method(__value)), &error_ty);
            quote!(.blocking_setter(|__thing: &Self, __value| #call))
        } else {
            let call = out.wrap(quote!(__thing.#method(__value).await), &error_ty);
            quote!(.setter(|__thing: ::std::sync::Arc<Self>, __value| async move { #call }))
        }
    });
    let resetter = property.resetter.as_ref().map(|accessor| {
        let method = &accessor.method.sig.ident;
        let out = Output::of(&accessor.method.sig.output);
        if accessor.blocking {
            let call = out.wrap(quote!(__thing.#method()), &error_ty);
            quote!(.blocking_resetter(|__thing: &Self| #call))
        } else {
            let call = out.wrap(quote!(__thing.#method().await), &error_ty);
            quote!(.resetter(|__thing: ::std::sync::Arc<Self>| async move { #call }))
        }
    });
    let options = &property.options;
    let default = options.default.as_ref().map(|d| {
        let d = default_expr(d);
        quote!(.default_value(#d))
    });
    let constraints = options.constraints().map(|c| quote!(.constraints(#c)));
    let readonly = options.readonly.then(|| quote!(.read_only()));
    let setting = property.setting.then(|| quote!(.setting()));
    let described = describe(
        docstring(&property.method.attrs).as_deref(),
        &options.described,
        true,
    );
    quote! {
        .property(#name, #make #setter #resetter #default #constraints #readonly #setting #described)
    }
}

struct GeneratedAction {
    definition: TokenStream,
    input: TokenStream,
    wrapper: (TokenStream, TokenStream),
}

fn action_tokens(
    action: &Action,
    type_ident: &Ident,
    self_ty: &Type,
    asserts: &mut Vec<TokenStream>,
) -> GeneratedAction {
    let method = &action.method.sig.ident;
    let name = &action.name;
    let output = Output::of(&action.method.sig.output);
    let error_ty = quote!(::teta_wot::ActionError);
    if !matches!(output, Output::Unit) {
        let value = output.value_type();
        asserts
            .push(quote_spanned!(value.span()=> ::teta_wot::__private::action_output::<#value>();));
    }

    // The input model.
    let fields: Vec<&Param> = action
        .params
        .iter()
        .filter(|p| matches!(p, Param::Field { .. }))
        .collect();
    let whole = action.params.iter().find_map(|p| match p {
        Param::Whole { ty, .. } => Some(ty.clone()),
        _ => None,
    });
    let input_ident = format_ident!("__WotInput_{}_{}", type_ident, name.replace('-', "_"));
    let (input_ty, input_def) = if let Some(ty) = whole {
        asserts.push(quote_spanned!(ty.span()=> ::teta_wot::__private::action_parameter::<#ty>();));
        (ty.to_token_stream(), TokenStream::new())
    } else if fields.is_empty() {
        (quote!(::teta_wot::NoInput), TokenStream::new())
    } else {
        let mut defaults = Vec::new();
        let field_defs = fields
            .iter()
            .map(|p| {
                let Param::Field {
                    ident,
                    ty,
                    default,
                    description,
                } = p
                else {
                    unreachable!()
                };
                asserts.push(
                    quote_spanned!(ty.span()=> ::teta_wot::__private::action_parameter::<#ty>();),
                );
                let default_attr = match default {
                    None => TokenStream::new(),
                    Some(None) => quote!(#[serde(default)]),
                    Some(Some(expr)) => {
                        let function = format_ident!("__default_{}", ident);
                        let expr = default_expr(expr);
                        defaults.push(quote! {
                            fn #function() -> #ty { #expr }
                        });
                        let path = format!("{input_ident}::{function}");
                        quote!(#[serde(default = #path)])
                    }
                };
                let description = description
                    .as_ref()
                    .map(|d| quote!(#[schemars(description = #d)]));
                quote! {
                    #default_attr
                    #description
                    #ident: #ty
                }
            })
            .collect::<Vec<_>>();
        let definition = quote! {
            #[derive(
                ::teta_wot::__private::serde::Serialize,
                ::teta_wot::__private::serde::Deserialize,
                ::teta_wot::__private::schemars::JsonSchema,
            )]
            #[serde(crate = "::teta_wot::__private::serde", deny_unknown_fields)]
            #[schemars(crate = "::teta_wot::__private::schemars")]
            #[allow(non_camel_case_types, missing_docs)]
            #[doc(hidden)]
            struct #input_ident {
                #(#field_defs,)*
            }

            #[allow(non_snake_case)]
            impl #input_ident {
                #(#defaults)*
            }
        };
        (input_ident.to_token_stream(), definition)
    };

    // The call, shared by the invocation handler and the in-process wrapper.
    let has_ctx = action.params.iter().any(|p| matches!(p, Param::Ctx));
    // Injected values are taken from the server before the call.
    let mut injections = Vec::new();
    let args: Vec<TokenStream> = action
        .params
        .iter()
        .enumerate()
        .map(|(index, p)| match p {
            Param::Ctx if action.options.blocking => quote!(__ctx_arg),
            Param::Ctx => quote!(__ctx),
            Param::Field { ident, .. } => quote!(__input.#ident),
            Param::Whole { .. } => quote!(__input),
            Param::Injected(service) => {
                let value = format_ident!("__injected_{}", index);
                let make = match service {
                    Some(service) => quote! {
                        ::teta_wot::Dep::new(__ctx.server().service::<#service>().ok_or_else(|| {
                            ::teta_wot::ActionError::from(::teta_wot::__private::anyhow::anyhow!(
                                "the service `{}` isn't registered with the server",
                                ::core::any::type_name::<#service>()
                            ))
                        })?)
                    },
                    None => quote!(::core::clone::Clone::clone(__ctx.server())),
                };
                injections.push(quote!(let #value = #make;));
                quote!(#value)
            }
        })
        .collect();
    let requires: Vec<TokenStream> = action
        .params
        .iter()
        .filter_map(|p| match p {
            Param::Injected(Some(service)) => Some(quote!(.requires_service::<#service>())),
            _ => None,
        })
        .collect();
    let call = if action.options.blocking {
        let ctx_arg = has_ctx.then(|| quote!(let __ctx_arg = ::core::clone::Clone::clone(&__ctx);));
        quote!({
            #(#injections)*
            #ctx_arg
            __ctx.blocking(move |_| __thing.#method(#(#args),*)).await
        })
    } else {
        quote!({
            #(#injections)*
            __thing.#method(#(#args),*).await
        })
    };
    let body = output.wrap(call, &error_ty);
    let handler = quote! {
        |__thing: ::std::sync::Arc<#self_ty>, __ctx: ::teta_wot::ActionCtx, __input: #input_ty| async move { #body }
    };

    let options = &action.options;
    let described = describe(
        docstring(&action.method.attrs).as_deref(),
        &options.described,
        false,
    );
    let retention = options.retention.as_ref().map(|r| {
        quote!(.retention(::std::time::Duration::from_secs_f64(::core::convert::Into::<f64>::into(#r))))
    });
    let definition = quote! {
        .action(#name, ::teta_wot::Action::new(#handler) #described #retention)
        #(#requires)*
    };

    // The in-process wrapper.
    let wrapper_params: Vec<TokenStream> = action
        .params
        .iter()
        .filter_map(|p| match p {
            Param::Ctx | Param::Injected(_) => None,
            Param::Field { ident, ty, .. } | Param::Whole { ident, ty } => {
                Some(quote!(#ident: #ty))
            }
        })
        .collect();
    let make_input = match action
        .params
        .iter()
        .find(|p| matches!(p, Param::Whole { .. }))
    {
        Some(Param::Whole { ident, .. }) => quote!(#ident),
        _ if fields.is_empty() => {
            quote!(<::teta_wot::NoInput as ::core::default::Default>::default())
        }
        _ => {
            let names = fields.iter().map(|p| match p {
                Param::Field { ident, .. } => ident,
                _ => unreachable!(),
            });
            quote!(#input_ident { #(#names),* })
        }
    };
    let docs: Vec<&Attribute> = action
        .method
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc") && matches!(a.meta, Meta::NameValue(_)))
        .collect();
    let doc_line = format!("Calls the `{name}` action in-process (see `wot::ThingRef`).");
    let docs = if docs.is_empty() {
        quote!(#[doc = #doc_line])
    } else {
        quote!(#(#docs)*)
    };
    // A boxed future rather than `impl Future`: an `impl Trait` return in a
    // public trait is an associated type, where the Thing's private types
    // would be a hard error.
    let value = output.value_type();
    let returns = quote! {
        ::core::pin::Pin<::std::boxed::Box<
            dyn ::core::future::Future<Output = ::core::result::Result<#value, ::teta_wot::ActionError>>
                + ::core::marker::Send
        >>
    };
    let signature = quote! {
        #docs
        fn #method(&self, #(#wrapper_params),*) -> #returns
    };
    let implementation = quote! {
        fn #method(&self, #(#wrapper_params),*) -> #returns {
            let __this = ::core::clone::Clone::clone(self);
            ::std::boxed::Box::pin(async move {
                ::teta_wot::ThingRef::__call(&__this, #name, #make_input, #handler).await
            })
        }
    };

    GeneratedAction {
        definition,
        input: input_def,
        wrapper: (signature, implementation),
    }
}

fn actions_trait(
    type_ident: &Ident,
    self_ty: &Type,
    wrappers: &[(TokenStream, TokenStream)],
) -> TokenStream {
    if wrappers.is_empty() {
        return TokenStream::new();
    }
    let trait_ident = format_ident!("{}Actions", type_ident);
    let doc = format!(
        "In-process calls to the actions of [`{type_ident}`], through a [`ThingRef`](::teta_wot::ThingRef): \
         the input is validated and the global lock is held as for an HTTP request, and the action \
         runs in the caller's invocation (generated by `#[thing_impl]`)."
    );
    let signatures = wrappers.iter().map(|(s, _)| s);
    let implementations = wrappers.iter().map(|(_, i)| i);
    quote! {
        #[doc = #doc]
        #[allow(private_interfaces, private_bounds)]
        pub trait #trait_ident {
            #(#signatures;)*
        }

        #[automatically_derived]
        impl #trait_ident for ::teta_wot::ThingRef<#self_ty> {
            #(#implementations)*
        }
    }
}

fn endpoint_tokens(endpoint: &Endpoint) -> TokenStream {
    let method = &endpoint.method.sig.ident;
    let http_method = &endpoint.http_method;
    let path = &endpoint.path;
    let args: Vec<Ident> = typed_params(&endpoint.method)
        .enumerate()
        .map(|(i, _)| format_ident!("__arg{}", i))
        .collect();
    let types = typed_params(&endpoint.method).map(|t| &t.ty);
    let description = docstring(&endpoint.method.attrs).map(|d| quote!(.description(#d)));
    quote! {
        .endpoint(::teta_wot::http::Endpoint::new(#http_method, #path, |__thing: ::std::sync::Arc<Self>| {
            move |#(#args: #types),*| {
                let __thing = ::std::sync::Arc::clone(&__thing);
                async move { __thing.#method(#(#args),*).await }
            }
        }) #description)
    }
}

fn hook_tokens(method: &ImplItemFn, start: bool) -> TokenStream {
    let ident = &method.sig.ident;
    let takes_ctx = typed_params(method).next().is_some();
    let args = takes_ctx.then(|| quote!(, ctx));
    let ignore = (!takes_ctx).then(|| quote!(let _ = ctx;));
    let call = quote!(<Self>::#ident(&self #args).await);
    if start {
        let body = Output::of(&method.sig.output)
            .wrap(call, &quote!(::teta_wot::__private::anyhow::Error));
        quote! {
            fn start(
                self: ::std::sync::Arc<Self>,
                ctx: ::teta_wot::ThingCtx,
            ) -> ::teta_wot::__private::BoxFuture<::teta_wot::__private::anyhow::Result<()>> {
                #ignore
                ::std::boxed::Box::pin(async move { #body })
            }
        }
    } else {
        quote! {
            fn stop(
                self: ::std::sync::Arc<Self>,
                ctx: ::teta_wot::ThingCtx,
            ) -> ::teta_wot::__private::BoxFuture<()> {
                #ignore
                ::std::boxed::Box::pin(async move { #call; })
            }
        }
    }
}
