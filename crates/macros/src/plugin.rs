use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::support::{attrs, diag, utils};

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

    let plugin_attrs = attrs::parse_attrs(&input.attrs, "plugin")?;
    let key = plugin_attrs.require_string("key", struct_name)?;
    let name = plugin_attrs.require_string("name", struct_name)?;
    let description = plugin_attrs
        .get_string("description")
        .or_else(|| Some(utils::doc_string(&input.attrs)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.clone());
    let version = plugin_attrs.get_int("version").unwrap_or(1) as u32;
    let group = plugin_attrs.get_list("group").unwrap_or_default();

    match &input.data {
        Data::Struct(_) => {}
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Plugin derive can only be used on structs",
            ));
        }
    };

    let group_items: Vec<_> = group.iter().map(|g| quote!(#g.to_string())).collect();

    let expanded = quote! {
        impl #impl_generics ::nebula_plugin::Plugin for #struct_name #ty_generics #where_clause {
            fn metadata(&self) -> &::nebula_plugin::PluginMetadata {
                use ::std::sync::OnceLock;

                static METADATA: OnceLock<::nebula_plugin::PluginMetadata> = OnceLock::new();
                METADATA.get_or_init(|| {
                    ::nebula_plugin::PluginMetadata::builder(#key, #name)
                        .description(#description)
                        .version(#version)
                        .group(vec![#(#group_items),*])
                        .build()
                        .expect("invalid plugin metadata")
                })
            }

            fn register(&self, _components: &mut ::nebula_plugin::PluginComponents) {}
        }
    };

    Ok(expanded.into())
}
