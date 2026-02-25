//! Parsed `#[action(...)]` attributes.
//!
//! [`ActionAttrs`] mirrors [`ActionMetadata`] fields that can be specified
//! via attributes, plus credential/resource components.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

use crate::support::attrs;

/// Parsed action container attributes.
///
/// Maps to [`ActionMetadata`] fields plus `ActionComponents` dependencies.
///
/// [`ActionMetadata`]: https://docs.rs/nebula-action/latest/nebula_action/struct.ActionMetadata.html
#[derive(Debug, Clone)]
pub struct ActionAttrs {
    // ── ActionMetadata fields ───────────────────────────────────────────────
    /// Unique key (e.g. `"http.request"`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Parsed version major.
    pub version_major: u32,
    /// Parsed version minor.
    pub version_minor: u32,
    /// Optional parameters type (e.g. `parameters = HttpConfig`).
    pub parameters: Option<Type>,

    // ── ActionComponents (credential/resource dependencies) ──────────────────
    /// Single credential type.
    pub credential: Option<Type>,
    /// Multiple credential types.
    pub credentials: Vec<Type>,
    /// Single resource type.
    pub resource: Option<Type>,
    /// Multiple resource types.
    pub resources: Vec<Type>,
}

impl ActionAttrs {
    /// Parse from `#[action(...)]` attribute args.
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

        let version_str = attr_args
            .get_string("version")
            .unwrap_or_else(|| "1.0".to_string());
        let (version_major, version_minor) = parse_version(&version_str)?;

        let parameters = attr_args.get_type("parameters")?;

        let credential = attr_args.get_type_skip_string("credential")?;
        let credentials = attr_args.get_type_list("credentials")?;

        let resource = attr_args.get_type_skip_string("resource")?;
        let resources = attr_args.get_type_list("resources")?;

        Ok(Self {
            key,
            name,
            description,
            version_major,
            version_minor,
            parameters,
            credential,
            credentials,
            resource,
            resources,
        })
    }

    /// All credential types (single + list).
    fn all_credentials(&self) -> Vec<&Type> {
        let mut out = Vec::new();
        if let Some(ref t) = self.credential {
            out.push(t);
        }
        out.extend(self.credentials.iter());
        out
    }

    /// All resource types (single + list).
    fn all_resources(&self) -> Vec<&Type> {
        let mut out = Vec::new();
        if let Some(ref t) = self.resource {
            out.push(t);
        }
        out.extend(self.resources.iter());
        out
    }

    /// Generate `ActionMetadata` initialization expression.
    pub fn metadata_init_expr(&self) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let major = self.version_major;
        let minor = self.version_minor;

        let params_expr = match &self.parameters {
            Some(ty) => quote! {
                .with_parameters(<#ty>::parameters())
            },
            None => quote! {},
        };

        quote! {
            ::nebula_action::metadata::ActionMetadata::new(#key, #name, #description)
                .with_version(#major, #minor)
                #params_expr
        }
    }

    /// Generate `ActionComponents` expression.
    pub fn components_expr(&self) -> TokenStream2 {
        let cred_refs: Vec<TokenStream2> = self
            .all_credentials()
            .iter()
            .map(|ty| quote! { ::nebula_credential::CredentialRef::of::<#ty>() })
            .collect();

        let res_refs: Vec<TokenStream2> = self
            .all_resources()
            .iter()
            .map(|ty| quote! { ::nebula_resource::ResourceRef::of::<#ty>() })
            .collect();

        let has_creds = !cred_refs.is_empty();
        let has_res = !res_refs.is_empty();

        if !has_creds && !has_res {
            quote! { ::nebula_action::ActionComponents::new() }
        } else if has_creds && !has_res {
            quote! {
                ::nebula_action::ActionComponents::new()
                    #(.credential(#cred_refs))*
            }
        } else if !has_creds && has_res {
            quote! {
                ::nebula_action::ActionComponents::new()
                    #(.resource(#res_refs))*
            }
        } else {
            quote! {
                ::nebula_action::ActionComponents::new()
                    #(.credential(#cred_refs))*
                    #(.resource(#res_refs))*
            }
        }
    }
}

fn parse_version(version: &str) -> Result<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "empty version"))?
        .parse::<u32>()
        .map_err(|_| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "invalid version format, expected `major.minor` (e.g. `1.0`)",
            )
        })?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().map_err(|_| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "invalid version format, expected `major.minor` (e.g. `1.0`)",
        )
    })?;
    Ok((major, minor))
}
