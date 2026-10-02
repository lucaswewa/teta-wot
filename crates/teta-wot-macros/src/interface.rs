//! `#[interface]`: a trait that slots can target (`Slot<dyn Trait>`),
//! provided by any Thing that declares it.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{ItemTrait, TypeParamBound};

use crate::common::error;

pub fn expand(args: TokenStream, item: ItemTrait) -> syn::Result<TokenStream> {
    if !args.is_empty() {
        return Err(error(args.span(), "`#[interface]` takes no arguments"));
    }
    if !item.generics.params.is_empty() {
        return Err(error(item.generics.span(), "an interface can't be generic"));
    }
    let has = |name: &str| {
        item.supertraits.iter().any(|bound| match bound {
            TypeParamBound::Trait(t) => t.path.segments.last().is_some_and(|s| s.ident == name),
            _ => false,
        })
    };
    if !has("Send") || !has("Sync") {
        return Err(error(
            item.ident.span(),
            "an interface must have `Send + Sync` supertraits: Things use it from any thread",
        ));
    }
    let ident = &item.ident;
    let name = format!("dyn {ident}");
    Ok(quote! {
        #item

        #[automatically_derived]
        impl ::teta_wot::SlotTarget for dyn #ident {
            type Handle = ::std::sync::Arc<dyn #ident>;

            fn resolve(
                thing: &::std::sync::Arc<::teta_wot::ThingHandle>,
            ) -> ::core::option::Option<::std::sync::Arc<dyn #ident>> {
                ::teta_wot::slots::resolve_interface::<dyn #ident>(thing)
            }

            fn target_name() -> ::std::string::String {
                ::std::string::String::from(#name)
            }
        }
    })
}
