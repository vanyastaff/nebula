//! Parsed `#[action(...)]` attributes.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

/// Parsed action container attributes.
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
    /// Optional parameters type (e.g. `parameters = HttpConfig`).
    pub parameters: Option<Type>,
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

        let version_str = attr_args
            .get_string("version")
            .unwrap_or_else(|| "1.0".to_string());
        let (version_major, version_minor, version_patch) = parse_version(&version_str)?;

        let parameters = attr_args.get_type("parameters")?;

        let credential = attr_args.get_type_skip_string("credential")?;
        let credentials = attr_args.get_type_list("credentials")?;

        let resource = attr_args.get_type_skip_string("resource")?;
        let resources = attr_args.get_type_list("resources")?;

        // Check for duplicate credential types.
        {
            let mut all = Vec::new();
            if let Some(ref t) = credential {
                all.push(t);
            }
            all.extend(credentials.iter());

            let mut seen = std::collections::HashSet::new();
            for ty in &all {
                let s = quote::quote!(#ty).to_string();
                if !seen.insert(s.clone()) {
                    return Err(syn::Error::new_spanned(
                        ty,
                        format!(
                            "duplicate credential type `{s}` \
                             — each type can only be declared once per action"
                        ),
                    ));
                }
            }
        }

        Ok(Self {
            key,
            name,
            description,
            version_major,
            version_minor,
            version_patch,
            parameters,
            credential,
            credentials,
            resource,
            resources,
        })
    }

    fn all_credentials(&self) -> Vec<&Type> {
        let mut out = Vec::new();
        if let Some(ref t) = self.credential {
            out.push(t);
        }
        out.extend(self.credentials.iter());
        out
    }

    fn all_resources(&self) -> Vec<&Type> {
        let mut out = Vec::new();
        if let Some(ref t) = self.resource {
            out.push(t);
        }
        out.extend(self.resources.iter());
        out
    }

    /// Generate `ActionMetadata` initialization expression.
    pub(crate) fn metadata_init_expr(&self) -> TokenStream2 {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let major = self.version_major;
        let minor = self.version_minor;
        let patch = self.version_patch;

        let params_expr = match &self.parameters {
            Some(ty) => quote! {
                .with_parameters(<#ty>::parameters())
            },
            None => quote! {},
        };

        quote! {
            ::nebula_action::metadata::ActionMetadata::new(
                ::nebula_core::ActionKey::new(#key).expect("invalid action key in #[action] attribute"),
                #name,
                #description,
            )
                .with_version_full(::semver::Version::new(#major, #minor, #patch))
                #params_expr
        }
    }

    /// Generate `DeclaresDependencies` impl expression.
    pub(crate) fn dependencies_impl_expr(
        &self,
        struct_name: &Ident,
        impl_generics: &syn::ImplGenerics<'_>,
        ty_generics: &syn::TypeGenerics<'_>,
        where_clause: Option<&syn::WhereClause>,
    ) -> TokenStream2 {
        let all_creds = self.all_credentials();
        let all_res = self.all_resources();

        let credential_calls: Vec<TokenStream2> = all_creds
            .iter()
            .map(|ty| {
                quote! {
                    .credential(
                        ::nebula_core::CredentialRequirement::new(
                            <#ty as ::nebula_core::CredentialLike>::KEY_STR,
                            ::std::any::TypeId::of::<#ty>(),
                            ::std::any::type_name::<#ty>(),
                        )
                    )
                }
            })
            .collect();

        let resource_calls: Vec<TokenStream2> = all_res
            .iter()
            .map(|ty| {
                quote! {
                    .resource(
                        ::nebula_core::ResourceRequirement::new(
                            <#ty as ::nebula_core::ResourceLike>::KEY_STR,
                            ::std::any::TypeId::of::<#ty>(),
                            ::std::any::type_name::<#ty>(),
                        )
                    )
                }
            })
            .collect();

        quote! {
            impl #impl_generics ::nebula_core::DeclaresDependencies for #struct_name #ty_generics #where_clause {
                fn dependencies() -> ::nebula_core::Dependencies {
                    ::nebula_core::Dependencies::new()
                        #(#credential_calls)*
                        #(#resource_calls)*
                }
            }
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
