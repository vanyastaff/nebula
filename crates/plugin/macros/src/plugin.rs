//! Plugin derive macro implementation.

use nebula_macro_support::{attrs, diag, utils};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::plugin_attrs::PluginAttrs;

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

    validate_struct(&input)?;

    let attr_args = attrs::parse_attrs(&input.attrs, "plugin")?;
    let description_fallback = utils::doc_string(&input.attrs);
    let description_fallback = if description_fallback.is_empty() {
        None
    } else {
        Some(description_fallback)
    };

    let attrs = PluginAttrs::parse(&attr_args, struct_name, description_fallback)?;
    let manifest_expr = attrs.manifest_builder_expr();

    let expanded = quote! {
        impl #impl_generics ::nebula_plugin::Plugin for #struct_name #ty_generics #where_clause {
            fn manifest(&self) -> &::nebula_plugin::PluginManifest {
                use ::std::sync::OnceLock;

                static MANIFEST: OnceLock<::nebula_plugin::PluginManifest> = OnceLock::new();
                MANIFEST.get_or_init(|| #manifest_expr)
            }
        }
    };

    Ok(expanded)
}

fn validate_struct(input: &DeriveInput) -> syn::Result<()> {
    match &input.data {
        Data::Struct(_) => Ok(()),
        _ => Err(syn::Error::new(
            input.ident.span(),
            "Plugin derive can only be used on structs",
        )),
    }
}
