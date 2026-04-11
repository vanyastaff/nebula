//! Parsed `#[action(...)]` attributes.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

/// Parsed action container attributes.
#[derive(Debug, Clone)]
pub struct ActionAttrs {
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
            ::nebula_action::metadata::ActionMetadata::new(
                ::nebula_core::ActionKey::new(#key).expect("invalid action key in #[action] attribute"),
                #name,
                #description,
            )
                .with_version(#major, #minor)
                #params_expr
        }
    }

    /// Generate `ActionDependencies` impl expression.
    pub fn dependencies_impl_expr(
        &self,
        struct_name: &Ident,
        impl_generics: &syn::ImplGenerics<'_>,
        ty_generics: &syn::TypeGenerics<'_>,
        where_clause: Option<&syn::WhereClause>,
    ) -> TokenStream2 {
        let all_creds = self.all_credentials();
        let all_res = self.all_resources();

        let credential_method = if all_creds.is_empty() {
            quote! {}
        } else {
            let ty = all_creds[0];
            quote! {
                fn credential() -> ::std::option::Option<::std::boxed::Box<dyn ::nebula_credential::AnyCredential>>
                where
                    Self: Sized,
                {
                    Some(::std::boxed::Box::new(<#ty as ::std::default::Default>::default()))
                }
            }
        };

        let resources_method = if all_res.is_empty() {
            quote! {}
        } else {
            let res_exprs: Vec<TokenStream2> = all_res
                .iter()
                .map(|ty| {
                    quote! {
                        ::std::boxed::Box::new(<#ty as ::std::default::Default>::default()) as ::std::boxed::Box<dyn ::nebula_resource::AnyResource>
                    }
                })
                .collect();
            quote! {
                fn resources() -> ::std::vec::Vec<::std::boxed::Box<dyn ::nebula_resource::AnyResource>>
                where
                    Self: Sized,
                {
                    vec![ #(#res_exprs),* ]
                }
            }
        };

        let credential_types_method = if all_creds.is_empty() {
            quote! {}
        } else {
            let type_ids: Vec<TokenStream2> = all_creds
                .iter()
                .map(|ty| {
                    quote! { ::std::any::TypeId::of::<#ty>() }
                })
                .collect();
            quote! {
                fn credential_types() -> ::std::vec::Vec<::std::any::TypeId>
                where
                    Self: Sized,
                {
                    vec![ #(#type_ids),* ]
                }
            }
        };

        quote! {
            impl #impl_generics ::nebula_action::ActionDependencies for #struct_name #ty_generics #where_clause {
                #credential_method
                #resources_method
                #credential_types_method
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
