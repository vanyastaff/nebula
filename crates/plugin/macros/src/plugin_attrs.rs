//! Parsed `#[plugin(...)]` attributes.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result};

use nebula_macro_support::attrs;

/// Parsed plugin container attributes.
#[derive(Debug, Clone)]
pub struct PluginAttrs {
    /// Unique key (e.g. `"http"`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Version number.
    pub version: u32,
    /// Group hierarchy for UI (e.g. `["network", "api"]`).
    pub group: Vec<String>,
}

impl PluginAttrs {
    /// Parse from `#[plugin(...)]` attribute args.
    pub fn parse(
        attr_args: &attrs::AttrArgs,
        struct_name: &Ident,
        description_fallback: Option<String>,
    ) -> Result<Self> {
        let key = attr_args.require_string("key", struct_name)?;
        let name = attr_args.require_string("name", struct_name)?;

        let description = attr_args
            .get_string("description")
            .or(description_fallback)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.clone());

        let version = attr_args.get_int("version").unwrap_or(1) as u32;
        let group = attr_args.get_list("group").unwrap_or_default();

        Ok(Self {
            key,
            name,
            description,
            version,
            group,
        })
    }

    /// Generate `PluginMetadata` builder expression.
    pub fn metadata_builder_expr(&self) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let version = self.version;
        let group_items: Vec<TokenStream2> =
            self.group.iter().map(|g| quote!(#g.to_string())).collect();

        quote! {
            ::nebula_plugin::PluginMetadata::builder(#key, #name)
                .description(#description)
                .version(#version)
                .group(vec![#(#group_items),*])
                .build()
                .expect("invalid plugin metadata")
        }
    }
}
