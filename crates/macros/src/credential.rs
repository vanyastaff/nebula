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

    // Optional: extends = SomeProtocol (must impl CredentialProtocol)
    let extends_type = cred_attrs.get_type("extends")?;

    // Optional explicit input type; when `extends` is set it defaults to ParameterValues
    let explicit_input = cred_attrs.get_type("input")?;

    // Optional explicit state type; when `extends` is set it defaults to <Protocol as CredentialProtocol>::State
    let explicit_state = cred_attrs.get_type("state")?;

    match &input.data {
        Data::Struct(_) => {}
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Credential derive can only be used on structs",
            ));
        }
    }

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
            <#proto as ::nebula_credential::traits::CredentialProtocol>::State
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
            <#proto as ::nebula_credential::traits::CredentialProtocol>::parameters()
        },
        None => quote! {
            ::nebula_parameter::collection::ParameterCollection::new()
        },
    };

    // Resolve initialize body
    let initialize_body = match &extends_type {
        Some(proto) => quote! {
            let state = <#proto as ::nebula_credential::traits::CredentialProtocol>::build_state(input)?;
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
