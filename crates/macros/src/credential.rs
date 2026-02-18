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

    let input_type = cred_attrs.get_type("input")?.ok_or_else(|| {
        diag::error_spanned(struct_name, "missing required attribute `input = Type`")
    })?;

    let state_type = cred_attrs.get_type("state")?.ok_or_else(|| {
        diag::error_spanned(struct_name, "missing required attribute `state = Type`")
    })?;

    match &input.data {
        Data::Struct(_) => {}
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Credential derive can only be used on structs",
            ));
        }
    };

    let expanded = quote! {
        #[::async_trait::async_trait]
        impl #impl_generics ::nebula_credential::Credential for #struct_name #ty_generics #where_clause {
            type Input = #input_type;
            type State = #state_type;

            fn description(&self) -> ::nebula_credential::core::CredentialDescription {
                use ::std::sync::OnceLock;

                static DESCRIPTION: OnceLock<::nebula_credential::core::CredentialDescription> = OnceLock::new();
                DESCRIPTION.get_or_init(|| {
                    ::nebula_credential::core::CredentialDescription {
                        key: #key.to_string(),
                        name: #name.to_string(),
                        description: #description.to_string(),
                        icon: None,
                        icon_url: None,
                        documentation_url: None,
                        properties: ::nebula_parameter::collection::ParameterCollection::new(),
                    }
                }).clone()
            }

            async fn initialize(
                &self,
                _input: &Self::Input,
                _ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<
                ::nebula_credential::core::result::InitializeResult<Self::State>,
                ::nebula_credential::core::CredentialError,
            > {
                ::std::todo!(
                    "implement `initialize` for credential `{}`",
                    stringify!(#struct_name)
                )
            }

            async fn refresh(
                &self,
                _state: &mut Self::State,
                _ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<(), ::nebula_credential::core::CredentialError> {
                Ok(())
            }

            async fn revoke(
                &self,
                _state: &mut Self::State,
                _ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<(), ::nebula_credential::core::CredentialError> {
                Ok(())
            }
        }
    };

    Ok(expanded.into())
}
