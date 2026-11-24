//! The identity attributes every component derive shares.
//!
//! `#[derive(Action)]` reads them from `#[action(...)]`, `#[derive(Resource)]`
//! from `#[resource(...)]`: `key`, `name`, `description`, `version` and
//! `config`. A derive adds its own kind's keys and checks for unknown ones.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, Result, Type};

use crate::attrs::AttrArgs;

/// The attribute keys this module reads.
pub const COMPONENT_KEYS: [&str; 5] = ["key", "name", "description", "version", "config"];

/// A component's identity and catalog fields, as written on its derive.
#[derive(Debug, Clone)]
pub struct ComponentAttrs {
    /// `Component::KEY`.
    pub key: String,
    /// Catalog name; defaults to the struct name.
    pub name: String,
    /// Catalog description; defaults to the struct's doc comment.
    pub description: String,
    /// Interface version, normalized to three components.
    pub version: semver::Version,
    /// `Component::Config`.
    pub config: Type,
}

impl ComponentAttrs {
    /// Reads the shared keys from `args`, the parsed `#[attribute(...)]` on
    /// `struct_name`.
    ///
    /// # Errors
    ///
    /// Returns a spanned error when `key` or `config` is missing or the
    /// version is not semver.
    pub fn parse(
        args: &AttrArgs,
        struct_name: &Ident,
        attribute: &str,
        doc_attributes: &[Attribute],
    ) -> Result<Self> {
        let key = args.require_string("key", struct_name)?;
        let name = args
            .get_string("name")
            .unwrap_or_else(|| struct_name.to_string());
        let description = args
            .get_string("description")
            .or_else(|| doc_string(doc_attributes))
            .unwrap_or_default();
        let version = parse_version(
            &args
                .get_string("version")
                .unwrap_or_else(|| "0.1.0".to_owned()),
        )?;
        let config = args.get_type("config")?.ok_or_else(|| {
            syn::Error::new_spanned(
                struct_name,
                format!(
                    "missing required attribute `config = SomeType` in #[{attribute}(...)] \
                     — it is the component's `Component::Config`; use `()` for none"
                ),
            )
        })?;
        Ok(Self {
            key,
            name,
            description,
            version,
            config,
        })
    }

    /// The `Component::metadata` body, building `Metadata` through `krate`.
    #[must_use]
    pub fn metadata_expr(&self, krate: &TokenStream) -> TokenStream {
        let name = &self.name;
        let description = &self.description;
        let version = self.version.to_string();
        quote! {
            #krate::Metadata::new(#krate::metadata_name!(#name), #description)
                .with_version_literal(#version)
        }
    }
}

/// The struct's doc comment, trimmed and joined by newlines; `None` when it
/// has none.
#[must_use]
pub fn doc_string(attributes: &[Attribute]) -> Option<String> {
    let lines: Vec<String> = attributes
        .iter()
        .filter_map(|attribute| {
            let syn::Meta::NameValue(name_value) = &attribute.meta else {
                return None;
            };
            if !name_value.path.is_ident("doc") {
                return None;
            }
            let syn::Expr::Lit(expression) = &name_value.value else {
                return None;
            };
            let syn::Lit::Str(documentation) = &expression.lit else {
                return None;
            };
            let line = documentation.value();
            let line = line.trim();
            (!line.is_empty()).then(|| line.to_owned())
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Parses `1`, `1.2` or `1.2.3` (with optional pre-release or build) as semver.
///
/// # Errors
///
/// Returns an error for an empty or non-semver string.
pub fn parse_version(version: &str) -> Result<semver::Version> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "empty version string; expected semver like `1.0` or `1.0.0`",
        ));
    }
    let mut normalized = trimmed.to_owned();
    if !trimmed.contains('-') && !trimmed.contains('+') {
        while normalized.split('.').count() < 3 {
            normalized.push_str(".0");
        }
    }
    semver::Version::parse(&normalized).map_err(|error| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("invalid version `{version}`: {error} — expected semver like `1.0` or `1.0.0`"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_versions_are_padded() {
        assert_eq!(parse_version("2").expect("valid").to_string(), "2.0.0");
        assert_eq!(parse_version("2.1").expect("valid").to_string(), "2.1.0");
        assert_eq!(
            parse_version("1.0.0-rc.1").expect("valid").to_string(),
            "1.0.0-rc.1"
        );
        assert!(parse_version("").is_err());
        assert!(parse_version("one").is_err());
    }
}
