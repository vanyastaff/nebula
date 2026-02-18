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

    let cred_attrs = attrs::parse_attrs(&input.attrs, "credential")?;
    let key = cred_attrs.require_string("key", struct_name)?;
    let name = cred_attrs.require_string("name", struct_name)?;
    let description = cred_attrs
        .get_string("description")
        .or_else(|| Some(utils::doc_string(&input.attrs)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.clone());

    // Optional: extends = SomeProtocol
    let extends_type = cred_attrs.get_type("extends")?;

    // Optional explicit input/state types
    let explicit_input = cred_attrs.get_type("input")?;
    let explicit_state = cred_attrs.get_type("state")?;

    // Sub-protocol attribute blocks
    let oauth2_attrs = attrs::parse_attrs(&input.attrs, "oauth2")?;
    let ldap_attrs = attrs::parse_attrs(&input.attrs, "ldap")?;

    // Detect protocol family by presence of sub-attribute blocks
    let has_oauth2 = !oauth2_attrs.items.is_empty();
    let has_ldap = !ldap_attrs.items.is_empty();

    match &input.data {
        Data::Struct(_) => {}
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Credential derive can only be used on structs",
            ));
        }
    }

    // ── FlowProtocol path (OAuth2) ─────────────────────────────────────────
    if has_oauth2 {
        return expand_oauth2_flow(
            struct_name,
            &impl_generics,
            &ty_generics,
            &where_clause,
            &extends_type,
            &key,
            &name,
            &description,
            &oauth2_attrs,
        );
    }

    // ── FlowProtocol path (LDAP) ───────────────────────────────────────────
    if has_ldap {
        return expand_ldap_flow(
            struct_name,
            &impl_generics,
            &ty_generics,
            &where_clause,
            &extends_type,
            &key,
            &name,
            &description,
            &ldap_attrs,
        );
    }

    // ── StaticProtocol path (default) ─────────────────────────────────────

    // Resolve Input type
    let resolved_input = match &explicit_input {
        Some(t) => quote! { #t },
        None if extends_type.is_some() => {
            quote! { ::nebula_parameter::values::ParameterValues }
        }
        None => {
            return Err(diag::error_spanned(
                struct_name,
                "missing required attribute `input = Type` (or use `extends = Protocol` to inherit input)",
            ));
        }
    };

    // Resolve State type
    let resolved_state = match (&explicit_state, &extends_type) {
        (Some(s), _) => quote! { #s },
        (None, Some(proto)) => quote! {
            <#proto as ::nebula_credential::traits::StaticProtocol>::State
        },
        (None, None) => {
            return Err(diag::error_spanned(
                struct_name,
                "missing required attribute `state = Type` (or use `extends = Protocol` to inherit state)",
            ));
        }
    };

    // Resolve properties expression for CredentialDescription
    let properties_expr = match &extends_type {
        Some(proto) => quote! {
            <#proto as ::nebula_credential::traits::StaticProtocol>::parameters()
        },
        None => quote! {
            ::nebula_parameter::collection::ParameterCollection::new()
        },
    };

    // Resolve initialize body
    let initialize_body = match &extends_type {
        Some(proto) => quote! {
            let state = <#proto as ::nebula_credential::traits::StaticProtocol>::build_state(input)?;
            ::std::result::Result::Ok(
                ::nebula_credential::core::result::InitializeResult::Complete(state)
            )
        },
        None => quote! {
            ::std::todo!(
                "implement `initialize` for credential `{}`",
                stringify!(#struct_name)
            )
        },
    };

    let expanded = quote! {
        #[::async_trait::async_trait]
        impl #impl_generics ::nebula_credential::traits::CredentialType
            for #struct_name #ty_generics #where_clause
        {
            type Input = #resolved_input;
            type State = #resolved_state;

            fn description() -> ::nebula_credential::core::CredentialDescription {
                use ::std::sync::OnceLock;

                static DESCRIPTION: OnceLock<::nebula_credential::core::CredentialDescription> =
                    OnceLock::new();
                DESCRIPTION.get_or_init(|| {
                    ::nebula_credential::core::CredentialDescription {
                        key: #key.to_string(),
                        name: #name.to_string(),
                        description: #description.to_string(),
                        icon: None,
                        icon_url: None,
                        documentation_url: None,
                        properties: #properties_expr,
                    }
                })
                .clone()
            }

            async fn initialize(
                &self,
                input: &Self::Input,
                _ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<
                ::nebula_credential::core::result::InitializeResult<Self::State>,
                ::nebula_credential::core::CredentialError,
            > {
                #initialize_body
            }
        }
    };

    Ok(expanded.into())
}

// ── OAuth2 FlowProtocol expansion ─────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn expand_oauth2_flow(
    struct_name: &syn::Ident,
    impl_generics: &syn::ImplGenerics<'_>,
    ty_generics: &syn::TypeGenerics<'_>,
    where_clause: &Option<&syn::WhereClause>,
    extends_type: &Option<syn::Type>,
    key: &str,
    name: &str,
    description: &str,
    oauth2_attrs: &attrs::AttrArgs,
) -> syn::Result<TokenStream> {
    // The protocol type — default to OAuth2Protocol
    let proto = match extends_type {
        Some(t) => quote! { #t },
        None => quote! { ::nebula_credential::protocols::OAuth2Protocol },
    };

    let auth_url = oauth2_attrs.require_string("auth_url", struct_name)?;
    let token_url = oauth2_attrs.require_string("token_url", struct_name)?;

    let scopes: Vec<String> = oauth2_attrs.get_list("scopes").unwrap_or_default();
    let scopes_tokens = quote! { vec![#(#scopes.to_string()),*] };

    let auth_style = match oauth2_attrs.get_ident_str("auth_style").as_deref() {
        Some("PostBody") => quote! { ::nebula_credential::protocols::AuthStyle::PostBody },
        _ => quote! { ::nebula_credential::protocols::AuthStyle::Header },
    };

    let pkce = oauth2_attrs.get_bool("pkce").unwrap_or(false);

    let expanded = quote! {
        #[::async_trait::async_trait]
        impl #impl_generics ::nebula_credential::traits::CredentialType
            for #struct_name #ty_generics #where_clause
        {
            type Input = ::nebula_parameter::values::ParameterValues;
            type State = ::nebula_credential::protocols::OAuth2State;

            fn description() -> ::nebula_credential::core::CredentialDescription {
                use ::std::sync::OnceLock;

                static DESCRIPTION: OnceLock<::nebula_credential::core::CredentialDescription> =
                    OnceLock::new();
                DESCRIPTION.get_or_init(|| {
                    ::nebula_credential::core::CredentialDescription {
                        key: #key.to_string(),
                        name: #name.to_string(),
                        description: #description.to_string(),
                        icon: None,
                        icon_url: None,
                        documentation_url: None,
                        properties: <#proto as ::nebula_credential::traits::FlowProtocol>::parameters(),
                    }
                })
                .clone()
            }

            async fn initialize(
                &self,
                input: &Self::Input,
                ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<
                ::nebula_credential::core::result::InitializeResult<Self::State>,
                ::nebula_credential::core::CredentialError,
            > {
                static CONFIG: ::std::sync::OnceLock<::nebula_credential::protocols::OAuth2Config> =
                    ::std::sync::OnceLock::new();
                let config = CONFIG.get_or_init(|| {
                    ::nebula_credential::protocols::OAuth2Config::authorization_code()
                        .auth_url(#auth_url)
                        .token_url(#token_url)
                        .scopes(#scopes_tokens)
                        .auth_style(#auth_style)
                        .pkce(#pkce)
                        .build()
                });
                <#proto as ::nebula_credential::traits::FlowProtocol>::initialize(config, input, ctx).await
            }
        }

        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// The statically configured [`OAuth2Config`] for this credential type.
            pub fn oauth2_config() -> &'static ::nebula_credential::protocols::OAuth2Config {
                static CONFIG: ::std::sync::OnceLock<::nebula_credential::protocols::OAuth2Config> =
                    ::std::sync::OnceLock::new();
                CONFIG.get_or_init(|| {
                    ::nebula_credential::protocols::OAuth2Config::authorization_code()
                        .auth_url(#auth_url)
                        .token_url(#token_url)
                        .scopes(#scopes_tokens)
                        .auth_style(#auth_style)
                        .pkce(#pkce)
                        .build()
                })
            }
        }
    };

    Ok(expanded.into())
}

// ── LDAP FlowProtocol expansion ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn expand_ldap_flow(
    struct_name: &syn::Ident,
    impl_generics: &syn::ImplGenerics<'_>,
    ty_generics: &syn::TypeGenerics<'_>,
    where_clause: &Option<&syn::WhereClause>,
    extends_type: &Option<syn::Type>,
    key: &str,
    name: &str,
    description: &str,
    ldap_attrs: &attrs::AttrArgs,
) -> syn::Result<TokenStream> {
    let proto = match extends_type {
        Some(t) => quote! { #t },
        None => quote! { ::nebula_credential::protocols::LdapProtocol },
    };

    let tls = match ldap_attrs.get_ident_str("tls").as_deref() {
        Some("Tls") => quote! { ::nebula_credential::protocols::TlsMode::Tls },
        Some("StartTls") => quote! { ::nebula_credential::protocols::TlsMode::StartTls },
        _ => quote! { ::nebula_credential::protocols::TlsMode::None },
    };

    let timeout_secs = ldap_attrs.get_int("timeout_secs").unwrap_or(30);

    let expanded = quote! {
        #[::async_trait::async_trait]
        impl #impl_generics ::nebula_credential::traits::CredentialType
            for #struct_name #ty_generics #where_clause
        {
            type Input = ::nebula_parameter::values::ParameterValues;
            type State = ::nebula_credential::protocols::LdapState;

            fn description() -> ::nebula_credential::core::CredentialDescription {
                use ::std::sync::OnceLock;

                static DESCRIPTION: OnceLock<::nebula_credential::core::CredentialDescription> =
                    OnceLock::new();
                DESCRIPTION.get_or_init(|| {
                    ::nebula_credential::core::CredentialDescription {
                        key: #key.to_string(),
                        name: #name.to_string(),
                        description: #description.to_string(),
                        icon: None,
                        icon_url: None,
                        documentation_url: None,
                        properties: <#proto as ::nebula_credential::traits::FlowProtocol>::parameters(),
                    }
                })
                .clone()
            }

            async fn initialize(
                &self,
                input: &Self::Input,
                ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<
                ::nebula_credential::core::result::InitializeResult<Self::State>,
                ::nebula_credential::core::CredentialError,
            > {
                static CONFIG: ::std::sync::OnceLock<::nebula_credential::protocols::LdapConfig> =
                    ::std::sync::OnceLock::new();
                let config = CONFIG.get_or_init(|| {
                    ::nebula_credential::protocols::LdapConfig {
                        tls: #tls,
                        timeout: ::std::time::Duration::from_secs(#timeout_secs),
                        ca_cert: None,
                    }
                });
                <#proto as ::nebula_credential::traits::FlowProtocol>::initialize(config, input, ctx).await
            }
        }
    };

    Ok(expanded.into())
}
