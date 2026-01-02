use anyhow::{Result, anyhow};
use proc_macro::TokenStream;
use proc_macro_crate::{Error, FoundCrate, crate_name};
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Ident, ItemFn, ReturnType, Type, parse_macro_input};

fn find_optional_crate(name: &str) -> Result<Option<FoundCrate>> {
    let result = crate_name(name);
    match result {
        Ok(result) => Ok(Some(result)),
        Err(err) => match err {
            Error::CrateNotFound {
                crate_name: _,
                path: _,
            } => Ok(None),
            _ => Err(err.into()),
        },
    }
}

fn aeth_entry_crate() -> Result<proc_macro2::TokenStream> {
    if let Some(found) = find_optional_crate("aeth-entry")? {
        let aeth_entry_crate = match found {
            FoundCrate::Itself => quote!(crate),
            FoundCrate::Name(name) => {
                let ident = Ident::new(name.as_str(), Span::call_site());
                quote!(::#ident)
            }
        };
        return Ok(aeth_entry_crate);
    }
    if let Some(found) = find_optional_crate("aeth")? {
        let aeth_entry_crate = match found {
            FoundCrate::Itself => quote!(crate::entry),
            FoundCrate::Name(name) => {
                let ident = Ident::new(name.as_str(), Span::call_site());
                quote!(::#ident::entry)
            }
        };
        return Ok(aeth_entry_crate);
    }
    Err(anyhow!("Neither `aeth` nor `aeth-entry` crate is found."))
}

fn main_result(mut item_fn: ItemFn) -> Result<TokenStream> {
    // Resolve the identifier of the aeth_entry crate.
    let aeth_entry_crate = aeth_entry_crate()?;

    // Make sure main function is well-formed.
    let main_ident = item_fn.sig.ident;
    if main_ident != "main" {
        return Err(anyhow!("Must only decorate the main function."));
    }
    if item_fn.sig.asyncness.is_none() {
        return Err(anyhow!("The main function must be async."));
    }

    // If the result is not present or `()` then we treat
    // the main function as returning unit type. Otherwise
    // we treat the main function as returning result.
    let return_unit = match &item_fn.sig.output {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => match ty.as_ref() {
            Type::Tuple(tpl) => tpl.elems.is_empty(),
            Type::Never(_) => {
                return Err(anyhow!(
                    "We need the main function to return, in order to shutdown the event loop."
                ));
            }
            _ => false,
        },
    };

    let inner_ident = Ident::new("__inner", main_ident.span());
    item_fn.sig.ident = inner_ident.clone();
    let vis = item_fn.vis;
    let sig = item_fn.sig;
    let body = item_fn.block;
    let future_block = if return_unit {
        quote! {
            async move {
                Ok(#inner_ident().await)
            }
        }
    } else {
        quote! {
            async move {
                Ok(#inner_ident().await?)
            }
        }
    };
    Ok(TokenStream::from(quote! {
        fn main() -> #aeth_entry_crate::Result<()> {
            #vis #sig {
                #body
            }
            #aeth_entry_crate::entrypoint(#future_block)
        }
    }))
}

/// Generate the main function for aeth-rs.
///
/// In order to use this macro, the user must
/// depend on either `aeth-entry` or `aeth`.
///
/// This procedural macro should wrap an **async**
/// main function. That is, an async function
/// with identifier `main` and optional return
/// type extending [`Result`].
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(item as ItemFn);
    let span = item_fn.span();
    match main_result(item_fn) {
        Ok(result) => result,
        Err(err) => {
            let err = syn::Error::new(span, err);
            err.to_compile_error().into()
        }
    }
}
