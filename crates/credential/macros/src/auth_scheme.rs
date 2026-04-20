//! `#[derive(AuthScheme)]` macro implementation.

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Entry point for `#[derive(AuthScheme)]`.
pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let attr_args = attrs::parse_attrs(&input.attrs, "auth_scheme")?;

    let pattern_ident = attr_args.get_ident("pattern").ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(AuthScheme)] requires `#[auth_scheme(pattern = Variant)]`",
        )
    })?;

    let expanded = quote! {
        impl #impl_generics ::nebula_core::AuthScheme
            for #struct_name #ty_generics #where_clause
        {
            fn pattern() -> ::nebula_core::AuthPattern {
                ::nebula_core::AuthPattern::#pattern_ident
            }
        }
    };

    Ok(expanded)
}
