use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{FnArg, Ident, ItemFn, Pat, parse_macro_input};

fn collect_fn_args(item_fn: &ItemFn) -> Vec<Ident> {
    item_fn
        .sig
        .inputs
        .iter()
        .filter_map(|input| {
            if let FnArg::Typed(typed) = input {
                Some(typed)
            } else {
                None
            }
        })
        .filter_map(|typed| {
            if let Pat::Ident(pat) = typed.pat.as_ref() {
                Some(pat.ident.clone())
            } else {
                None
            }
        })
        .collect()
}

fn is_mutable_method(item_fn: &ItemFn) -> Option<bool> {
    Some(
        item_fn
            .sig
            .inputs
            .iter()
            .filter_map(|input| {
                if let FnArg::Receiver(receiver) = input {
                    Some(receiver)
                } else {
                    None
                }
            })
            .next()?
            .mutability
            .is_some(),
    )
}

#[proc_macro_attribute]
pub fn forward_winit_window_method(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(item as ItemFn);
    let args = collect_fn_args(&item_fn);
    let mutable =
        is_mutable_method(&item_fn).expect("Must be a method of AccessWinitWindow trait extension");
    let vis = item_fn.vis;
    let sig = item_fn.sig;
    let name = sig.ident.clone();
    let method = Ident::new(
        if mutable {
            "map_winit_window_mut"
        } else {
            "map_winit_window"
        },
        Span::call_site(),
    );
    TokenStream::from(quote! {
        #[doc = concat!("Invoke [`winit::window::Window::", stringify!(#name), "`].")]
        #vis #sig {
            self.#method(move |v| v.#name(#(#args),*))
        }
    })
}

#[proc_macro_attribute]
pub fn forward_winit_active_event_loop_method(
    _attr: TokenStream,
    item: TokenStream,
) -> TokenStream {
    let item_fn = parse_macro_input!(item as ItemFn);
    let args = collect_fn_args(&item_fn);
    is_mutable_method(&item_fn)
        .expect("Must be a method of AccessWinitActiveEventLoop trait extension");
    let vis = item_fn.vis;
    let sig = item_fn.sig;
    let name = sig.ident.clone();
    TokenStream::from(quote! {
        #[doc = concat!("Invoke [`winit::event_loop::ActiveEventLoop::", stringify!(#name), "`].")]
        #vis #sig {
            self.run_with_active_event_loop(move |v| { v.#name(#(#args),*) })
                .await
        }
    })
}
