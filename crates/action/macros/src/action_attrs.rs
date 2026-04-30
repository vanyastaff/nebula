//! Parsed `#[action(...)]` attributes for the Variant A `#[derive(Action)]`.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

/// Parsed action container attributes (Variant A — ADR-0043 §6).
#[derive(Debug, Clone)]
pub(crate) struct ActionAttrs {
    /// Unique key (e.g. `"http.request"`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Parsed semver major component.
    pub version_major: u64,
    /// Parsed semver minor component.
    pub version_minor: u64,
    /// Parsed semver patch component.
    pub version_patch: u64,
    /// Required `Self::Input` type.
    pub input: Type,
    /// Required `Self::Output` type.
    pub output: Type,
}

impl ActionAttrs {
    /// Parse from `#[action(...)]` attribute args.
    pub(crate) fn parse(
        attr_args: &attrs::AttrArgs,
        struct_name: &Ident,
        description_fallback: Option<String>,
    ) -> Result<Self> {
        // Validate that all keys present are recognised. This produces clean
        // diagnostics for misspelled attribute names so authors get a hint
        // pointing at the bad key instead of a confusing parse error far
        // downstream.
        const ALLOWED: &[&str] = &["key", "name", "description", "version", "input", "output"];
        for item in &attr_args.items {
            let key = match item {
                attrs::AttrItem::KeyValue { key, .. }
                | attrs::AttrItem::Flag(key)
                | attrs::AttrItem::List { key, .. } => key,
            };
            if !ALLOWED.iter().any(|allowed| key == allowed) {
                return Err(syn::Error::new_spanned(
                    key,
                    format!(
                        "unknown attribute `{key}` in #[action(...)] \
                         — allowed keys: {}",
                        ALLOWED.join(", "),
                    ),
                ));
            }
        }

        let key = attr_args.require_string("key", struct_name)?;
        let name = attr_args
            .get_string("name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| struct_name.to_string());

        let description = attr_args
            .get_string("description")
            .or(description_fallback)
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

        let version_str = attr_args
            .get_string("version")
            .unwrap_or_else(|| "0.1.0".to_string());
        let (version_major, version_minor, version_patch) = parse_version(&version_str)?;

        let input = attr_args.get_type("input")?.ok_or_else(|| {
            syn::Error::new_spanned(
                struct_name,
                "missing required attribute `input = SomeType` \
                 — Variant A requires Self::Input to be specified",
            )
        })?;
        let output = attr_args.get_type("output")?.ok_or_else(|| {
            syn::Error::new_spanned(
                struct_name,
                "missing required attribute `output = SomeType` \
                 — Variant A requires Self::Output to be specified",
            )
        })?;

        Ok(Self {
            key,
            name,
            description,
            version_major,
            version_minor,
            version_patch,
            input,
            output,
        })
    }

    /// Generate `ActionMetadata` initialization expression.
    pub(crate) fn metadata_init_expr(&self) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let major = self.version_major;
        let minor = self.version_minor;
        let patch = self.version_patch;

        quote! {
            ::nebula_action::ActionMetadata::new(
                ::nebula_core::ActionKey::new(#key)
                    .expect("invalid action key in #[action] attribute"),
                #name,
                #description,
            )
                .with_version_full(::semver::Version::new(#major, #minor, #patch))
        }
    }
}

/// Parse a `#[action(version = "…")]` string into `(major, minor, patch)` components.
///
/// Accepts both the short `"X.Y"` shape (promoted to `X.Y.0`) and the full
/// semver `"X.Y.Z"` shape, plus any additional pre-release / build metadata
/// that `semver::Version::parse` understands. The parsed triple is emitted
/// into the action metadata expansion as `::semver::Version::new(...)`.
fn parse_version(version: &str) -> Result<(u64, u64, u64)> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "empty version string; expected semver like `1.0` or `1.0.0`",
        ));
    }

    // `semver::Version::parse` requires three components. Promote `X.Y` to
    // `X.Y.0` first so authors can keep writing the shorter form.
    let mut owned_buf;
    let normalized: &str = if trimmed.split('.').take(3).count() < 3
        && !trimmed.contains('-')
        && !trimmed.contains('+')
    {
        owned_buf = trimmed.to_owned();
        // Pad with ".0" until we have at least three dot-separated segments.
        while owned_buf.split('.').take(3).count() < 3 {
            owned_buf.push_str(".0");
        }
        owned_buf.as_str()
    } else {
        trimmed
    };

    let parsed = semver::Version::parse(normalized).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "invalid version `{version}`: {err} \
                 — expected semver like `1.0` or `1.0.0`"
            ),
        )
    })?;

    Ok((parsed.major, parsed.minor, parsed.patch))
}
