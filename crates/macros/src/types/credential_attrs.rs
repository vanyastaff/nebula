//! Parsed `#[credential(...)]` attributes.
//!
//! [`CredentialAttrs`] holds common credential metadata from the container attribute.
//! Sub-protocols (OAuth2, LDAP) use separate `#[oauth2(...)]` and `#[ldap(...)]` blocks.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

use crate::support::attrs;

/// Parsed credential container attributes.
///
/// Maps to `#[credential(...)]` on the struct.
#[derive(Debug, Clone)]
pub struct CredentialAttrs {
    /// Unique key (e.g. `"slack_oauth"`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Optional protocol to extend (e.g. `extends = OAuth2Protocol`).
    pub extends: Option<Type>,
    /// Explicit input type (for StaticProtocol).
    pub input: Option<Type>,
    /// Explicit state type (for StaticProtocol).
    pub state: Option<Type>,
}

impl CredentialAttrs {
    /// Parse from `#[credential(...)]` attribute args.
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

        let extends = attr_args.get_type("extends")?;
        let input = attr_args.get_type("input")?;
        let state = attr_args.get_type("state")?;

        Ok(Self {
            key,
            name,
            description,
            extends,
            input,
            state,
        })
    }

    /// Generate `CredentialDescription` fields expression (key, name, description, properties).
    pub fn description_fields_expr(&self, properties_expr: TokenStream2) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;

        quote! {
            key: #key.to_string(),
            name: #name.to_string(),
            description: #description.to_string(),
            icon: None,
            icon_url: None,
            documentation_url: None,
            properties: #properties_expr,
        }
    }
}
