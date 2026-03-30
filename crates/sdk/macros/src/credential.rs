//! Credential derive macro.
//!
//! TODO(v2): Rewrite to generate `nebula_credential::Credential` trait impls.
//! The v1 code generation (CredentialType, StaticProtocol, FlowProtocol) has
//! been removed. This macro currently emits a compile error directing users
//! to implement `Credential` manually.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    let expanded = quote! {
        compile_error!(
            concat!(
                "#[derive(Credential)] is being rewritten for the v2 Credential trait. ",
                "Please implement `nebula_credential::Credential` manually for `",
                stringify!(#struct_name),
                "` until the macro is updated."
            )
        );
    };

    expanded.into()
}
