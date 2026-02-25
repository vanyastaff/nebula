//! Plugin derive macro.
//!
//! Generates `Plugin` trait impl with `metadata()` and `register()`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::support::{attrs, diag, utils};
use crate::types::PluginAttrs;

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
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
    let metadata_expr = attrs.metadata_builder_expr();

    let expanded = quote! {
        impl #impl_generics ::nebula_plugin::Plugin for #struct_name #ty_generics #where_clause {
            fn metadata(&self) -> &::nebula_plugin::PluginMetadata {
                use ::std::sync::OnceLock;

                static METADATA: OnceLock<::nebula_plugin::PluginMetadata> = OnceLock::new();
                METADATA.get_or_init(|| #metadata_expr)
            }

            fn register(&self, _components: &mut ::nebula_plugin::PluginComponents) {}
        }
    };

    Ok(expanded.into())
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
