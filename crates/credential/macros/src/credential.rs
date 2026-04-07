//! Credential derive macro implementation (v2).

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

use nebula_macro_support::{attrs, diag};

/// Entry point for `#[derive(Credential)]`.
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

/// Parsed `#[credential(...)]` attributes for v2 derive.
struct CredentialAttrsV2 {
    key: String,
    name: String,
    scheme: syn::Type,
    protocol: syn::Type,
    icon: Option<String>,
    doc_url: Option<String>,
}

fn parse_credential_attrs(
    attr_args: &attrs::AttrArgs,
    struct_name: &syn::Ident,
) -> syn::Result<CredentialAttrsV2> {
    let key = attr_args.require_string("key", struct_name)?;
    let name = attr_args.require_string("name", struct_name)?;

    let scheme = attr_args.get_type("scheme")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(Credential)] requires `scheme = Type` attribute",
        )
    })?;

    let protocol = attr_args.get_type("protocol")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(Credential)] requires `protocol = Type` attribute",
        )
    })?;

    let icon = attr_args.get_string("icon");
    let doc_url = attr_args.get_string("doc_url");

    Ok(CredentialAttrsV2 {
        key,
        name,
        scheme,
        protocol,
        icon,
        doc_url,
    })
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    match &input.data {
        Data::Struct(data) => {
            if !matches!(&data.fields, Fields::Unit) {
                return Err(syn::Error::new(
                    input.ident.span(),
                    "#[derive(Credential)] requires a unit struct (e.g. `struct MyCredential;`)",
                ));
            }
        }
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "#[derive(Credential)] can only be used on structs",
            ));
        }
    }

    let attr_args = attrs::parse_attrs(&input.attrs, "credential")?;
    let attrs = parse_credential_attrs(&attr_args, struct_name)?;

    let key = &attrs.key;
    let name = &attrs.name;
    let scheme = &attrs.scheme;
    let protocol = &attrs.protocol;

    let icon_expr = match &attrs.icon {
        Some(icon) => quote! { ::std::option::Option::Some(#icon.to_owned()) },
        None => quote! { ::std::option::Option::None },
    };
    let doc_url_expr = match &attrs.doc_url {
        Some(url) => quote! { ::std::option::Option::Some(#url.to_owned()) },
        None => quote! { ::std::option::Option::None },
    };

    let expanded = quote! {
        impl #impl_generics ::nebula_credential::Credential
            for #struct_name #ty_generics #where_clause
        {
            type Scheme = #scheme;
            type State = #scheme;
            type Pending = ::nebula_credential::NoPendingState;

            const KEY: &'static str = #key;

            fn description() -> ::nebula_credential::CredentialDescription
            where
                Self: Sized,
            {
                ::nebula_credential::CredentialDescription {
                    key: #key.to_owned(),
                    name: #name.to_owned(),
                    description: #name.to_owned(),
                    icon: #icon_expr,
                    icon_url: ::std::option::Option::None,
                    documentation_url: #doc_url_expr,
                    properties: <#protocol as ::nebula_credential::StaticProtocol>::parameters(),
                    pattern: <#scheme as ::nebula_core::AuthScheme>::pattern(),
                }
            }

            fn parameters() -> ::nebula_parameter::ParameterCollection
            where
                Self: Sized,
            {
                <#protocol as ::nebula_credential::StaticProtocol>::parameters()
            }

            fn project(state: &#scheme) -> #scheme
            where
                Self: Sized,
            {
                state.clone()
            }

            fn resolve(
                values: &::nebula_parameter::values::ParameterValues,
                _ctx: &::nebula_credential::CredentialContext,
            ) -> impl ::std::future::Future<
                Output = ::std::result::Result<
                    ::nebula_credential::resolve::StaticResolveResult<#scheme>,
                    ::nebula_credential::CredentialError,
                >,
            > + ::std::marker::Send
            where
                Self: Sized,
            {
                async {
                    let scheme =
                        <#protocol as ::nebula_credential::StaticProtocol>::build(values)?;
                    ::std::result::Result::Ok(
                        ::nebula_credential::resolve::ResolveResult::Complete(scheme),
                    )
                }
            }
        }
    };

    Ok(expanded)
}
