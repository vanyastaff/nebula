//! Parsed `#[plugin(...)]` attributes.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use semver::Version;
use syn::{Ident, Result};

/// Parsed plugin container attributes.
#[derive(Debug, Clone)]
pub(crate) struct PluginAttrs {
    /// Unique key (e.g. `"http"`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Bundle semver version.
    pub version: Version,
    /// Group hierarchy for UI (e.g. `["network", "api"]`).
    pub group: Vec<String>,
}

impl PluginAttrs {
    /// Parse from `#[plugin(...)]` attribute args.
    pub(crate) fn parse(
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

        let version = match attr_args.get_string("version") {
            Some(raw) => raw.parse::<Version>().map_err(|e| {
                syn::Error::new(
                    struct_name.span(),
                    format!("invalid semver in #[plugin(version = \"{raw}\")]: {e}"),
                )
            })?,
            None => Version::new(1, 0, 0),
        };

        let group = attr_args.get_list("group").unwrap_or_default();

        Ok(Self {
            key,
            name,
            description,
            version,
            group,
        })
    }

    /// Generate `PluginManifest` builder expression.
    pub(crate) fn manifest_builder_expr(&self) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        // Emit `Version::new(major, minor, patch)`. Pre-release/build metadata is
        // not surfaced through the attribute macro — authors needing those can
        // construct the manifest manually. Validated at parse time above.
        let major = self.version.major;
        let minor = self.version.minor;
        let patch = self.version.patch;
        let group_items: Vec<TokenStream2> =
            self.group.iter().map(|g| quote!(#g.to_string())).collect();

        quote! {
            ::nebula_plugin::PluginManifest::builder(#key, #name)
                .description(#description)
                .version(::semver::Version::new(#major, #minor, #patch))
                .group(vec![#(#group_items),*])
                .build()
                .expect("invalid plugin manifest")
        }
    }
}
