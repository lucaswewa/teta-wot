//! What both macros share: doc comments, names, types and options.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::meta::ParseNestedMeta;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, ExprLit, GenericArgument, Ident, Lit, Meta, PathArguments, Type};

// The naming rules, shared with `wot-core`.
include!("../../teta-wot-core/src/reserved.rs");

/// Collects errors, to report several at once.
#[derive(Default)]
pub struct Errors(Option<syn::Error>);

impl Errors {
    pub fn push(&mut self, error: syn::Error) {
        match &mut self.0 {
            Some(first) => first.combine(error),
            None => self.0 = Some(error),
        }
    }

    pub fn push_result<T>(&mut self, result: syn::Result<T>) -> Option<T> {
        result.map_err(|e| self.push(e)).ok()
    }

    pub fn into_tokens(self) -> TokenStream {
        self.0
            .map(syn::Error::into_compile_error)
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// `Err` with every error collected, if any.
    pub fn finish(self) -> syn::Result<()> {
        self.0.map_or(Ok(()), Err)
    }
}

/// The doc comment on an item, as one string: the `#[doc]`
/// lines joined with newlines, with the indentation common to the non-blank
/// lines removed (so `/// text` gives `text`), and blank lines at the start
/// and end dropped. `None` if there is no doc comment.
pub fn docstring(attrs: &[Attribute]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) = &nv.value
        {
            lines.extend(text.value().split('\n').map(str::to_owned));
        }
    }
    let leading = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| leading(l))
        .min()?;
    let lines: Vec<&str> = lines.iter().map(|l| &l[leading(l).min(indent)..]).collect();
    let first = lines.iter().position(|l| !l.trim().is_empty())?;
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    Some(lines[first..=last].join("\n"))
}

/// Removes the attributes the macros consume.
pub fn strip_attrs(attrs: &mut Vec<Attribute>, names: &[&str]) {
    attrs.retain(|a| !names.iter().any(|n| a.path().is_ident(n)));
}

/// The affordance name for an identifier (`r#move` is `move`), checked.
pub fn affordance_name(ident: &Ident) -> syn::Result<String> {
    let name = ident.unraw().to_string();
    match affordance_name_problem(&name) {
        Some(problem) => Err(syn::Error::new(ident.span(), problem)),
        None => Ok(name),
    }
}

/// An item whose definition twice (once from each macro) is a compile error,
/// so two affordances with one name are reported even when one is a field
/// and the other a method.
pub fn collision_guard(ident: &Ident) -> TokenStream {
    let guard = quote::format_ident!(
        "__wot_duplicate_affordance_{}",
        ident.unraw(),
        span = ident.span()
    );
    // Spanned at the name, so that the error points at both definitions.
    quote_spanned! {ident.span()=>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        const #guard: () = ();
    }
}

/// The last segment of a path type.
pub fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(path) if path.qself.is_none() => path.path.segments.last(),
        Type::Group(group) => last_segment(&group.elem),
        Type::Paren(paren) => last_segment(&paren.elem),
        _ => None,
    }
}

/// Whether a type is written as `Name` or `…::Name`, with any arguments.
pub fn is_named(ty: &Type, name: &str) -> bool {
    last_segment(ty).is_some_and(|s| s.ident == name)
}

/// The type arguments of the last segment.
pub fn type_arguments(ty: &Type) -> Vec<&Type> {
    match last_segment(ty).map(|s| &s.arguments) {
        Some(PathArguments::AngleBracketed(args)) => args
            .args
            .iter()
            .filter_map(|a| match a {
                GenericArgument::Type(t) => Some(t),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `Some(T)` for `Wrapper<T>`.
pub fn unwrap_type<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    if is_named(ty, wrapper) {
        type_arguments(ty).first().copied()
    } else {
        None
    }
}

/// Whether a type is `()`.
pub fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

/// Whether a type borrows (a reference, anywhere in it).
pub fn borrows(ty: &Type) -> bool {
    match ty {
        Type::Reference(_) => true,
        Type::Tuple(t) => t.elems.iter().any(borrows),
        Type::Array(a) => borrows(&a.elem),
        Type::Slice(s) => borrows(&s.elem),
        Type::Paren(p) => borrows(&p.elem),
        Type::Group(g) => borrows(&g.elem),
        Type::Path(_) => type_arguments(ty).into_iter().any(borrows),
        _ => false,
    }
}

/// What a method returns, for deciding how to wrap its result.
pub enum Output<'a> {
    /// Nothing, or `()`.
    Unit,
    /// `Result<T, E>` (by the last segment's name, so aliases such as
    /// `anyhow::Result<T>` count too): the value type.
    Fallible(&'a Type),
    /// Any other type.
    Value(&'a Type),
}

impl<'a> Output<'a> {
    pub fn of(output: &'a syn::ReturnType) -> Self {
        match output {
            syn::ReturnType::Default => Output::Unit,
            syn::ReturnType::Type(_, ty) if is_unit(ty) => Output::Unit,
            syn::ReturnType::Type(_, ty) if is_named(ty, "Result") => {
                match type_arguments(ty).first() {
                    Some(value) => Output::Fallible(value),
                    None => Output::Value(ty),
                }
            }
            syn::ReturnType::Type(_, ty) => Output::Value(ty),
        }
    }

    /// The value type, `()` for `Unit`.
    pub fn value_type(&self) -> TokenStream {
        match self {
            Output::Unit => quote!(()),
            Output::Fallible(ty) | Output::Value(ty) => ty.to_token_stream(),
        }
    }

    /// Wraps an expression of the method's return type into
    /// `Result<value, error>`, converting a fallible method's error with
    /// `Into`.
    pub fn wrap(&self, call: TokenStream, error: &TokenStream) -> TokenStream {
        match self {
            Output::Unit => quote! {{
                #call;
                ::core::result::Result::Ok::<(), #error>(())
            }},
            Output::Fallible(_) => quote! {
                #call.map_err(::core::convert::Into::<#error>::into)
            },
            Output::Value(_) => quote! {
                ::core::result::Result::Ok::<_, #error>(#call)
            },
        }
    }
}

/// A default value expression: string literals become `From::from("…")`,
/// so that `default = "abc"` works for a `String`.
pub fn default_expr(expr: &Expr) -> TokenStream {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(_), ..
        }) => quote_spanned!(expr.span()=> ::core::convert::From::from(#expr)),
        _ => expr.to_token_stream(),
    }
}

/// Options that describe an affordance in the TD.
#[derive(Default)]
pub struct Described {
    pub title: Option<Expr>,
    pub description: Option<Expr>,
    pub unit: Option<Expr>,
    pub semantic_types: Vec<Expr>,
    pub global_lock: Option<Expr>,
}

/// The options of `#[property(…)]`, `#[action(…)]` and the other method
/// attributes. `allowed` lists which the attribute accepts.
#[derive(Default)]
pub struct Options {
    pub readonly: bool,
    pub blocking: bool,
    pub default: Option<Expr>,
    pub constraints: Vec<(Ident, Expr)>,
    pub retention: Option<Expr>,
    pub described: Described,
}

const CONSTRAINTS: [&str; 9] = [
    "ge",
    "gt",
    "le",
    "lt",
    "multiple_of",
    "min_length",
    "max_length",
    "pattern",
    "allow_inf_nan",
];

impl Options {
    pub fn parse(attr: &Attribute, allowed: &[&str]) -> syn::Result<Self> {
        let mut options = Options::default();
        if matches!(attr.meta, Meta::Path(_)) {
            return Ok(options);
        }
        let kind = attr
            .path()
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(|i| i.unraw().to_string())
                .unwrap_or_default();
            let key = if key == "read_only" {
                "readonly".to_owned()
            } else {
                key
            };
            let accepted = allowed.contains(&key.as_str())
                || (CONSTRAINTS.contains(&key.as_str()) && allowed.contains(&"constraints"));
            if !accepted {
                let mut names: Vec<&str> = allowed
                    .iter()
                    .copied()
                    .filter(|a| *a != "constraints")
                    .collect();
                if allowed.contains(&"constraints") {
                    names.extend(CONSTRAINTS);
                }
                return Err(meta.error(format!(
                    "unknown option for `#[{kind}]`; expected one of: {}",
                    names.join(", ")
                )));
            }
            options.apply(&key, &meta)
        })?;
        Ok(options)
    }

    fn apply(&mut self, key: &str, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        match key {
            "readonly" => self.readonly = true,
            "blocking" => self.blocking = true,
            "default" => self.default = Some(meta.value()?.parse()?),
            "retention" => self.retention = Some(meta.value()?.parse()?),
            "title" => self.described.title = Some(meta.value()?.parse()?),
            "description" => self.described.description = Some(meta.value()?.parse()?),
            "unit" => self.described.unit = Some(meta.value()?.parse()?),
            "semantic_type" => self.described.semantic_types.push(meta.value()?.parse()?),
            "global_lock" => self.described.global_lock = Some(meta.value()?.parse()?),
            constraint => {
                let ident = Ident::new(constraint, meta.path.span());
                self.constraints.push((ident, meta.value()?.parse()?));
            }
        }
        Ok(())
    }

    /// `Constraints::new().ge(…)…`, if there are any.
    pub fn constraints(&self) -> Option<TokenStream> {
        if self.constraints.is_empty() {
            return None;
        }
        let calls = self
            .constraints
            .iter()
            .map(|(name, value)| quote!(.#name(#value)));
        Some(quote!(::teta_wot::Constraints::new() #(#calls)*))
    }
}

/// Builder calls for the doc comment and the descriptive options, in the
/// order that lets explicit options win.
pub fn describe(doc: Option<&str>, described: &Described, with_unit: bool) -> TokenStream {
    let doc = doc.map(|d| quote!(.doc(#d)));
    let title = described.title.as_ref().map(|t| quote!(.title(#t)));
    let description = described
        .description
        .as_ref()
        .map(|d| quote!(.description(#d)));
    let unit = described
        .unit
        .as_ref()
        .filter(|_| with_unit)
        .map(|u| quote!(.unit(#u)));
    let types = described
        .semantic_types
        .iter()
        .map(|t| quote!(.semantic_type(#t)));
    let lock = described
        .global_lock
        .as_ref()
        .map(|l| quote!(.global_lock(#l)));
    quote!(#doc #title #description #unit #(#types)* #lock)
}

/// An error at a span.
pub fn error(span: Span, message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(span, message)
}
