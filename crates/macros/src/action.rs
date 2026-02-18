use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

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

    let action_attrs = attrs::parse_attrs(&input.attrs, "action")?;
    let key = action_attrs.require_string("key", struct_name)?;
    let name = action_attrs.require_string("name", struct_name)?;
    let description = action_attrs
        .get_string("description")
        .or_else(|| Some(utils::doc_string(&input.attrs)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.clone());

    let version = action_attrs
        .get_string("version")
        .unwrap_or_else(|| "1.0".to_string());
    let (version_major, version_minor) = parse_version(&version)?;

    let action_type = action_attrs
        .get_string("action_type")
        .unwrap_or_else(|| "process".to_string());
    let isolation = action_attrs
        .get_string("isolation")
        .unwrap_or_else(|| "none".to_string());
    let credential = action_attrs.get_string("credential");

    let action_type_variant = parse_action_type(&action_type)?;
    let isolation_level = parse_isolation_level(&isolation)?;

    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Action derive can only be used on structs",
            ));
        }
    };

    validate_field_attrs(fields)?;

    let metadata_init = generate_metadata_init(
        &key,
        &name,
        &description,
        version_major,
        version_minor,
        action_type_variant,
        isolation_level,
        credential.as_deref(),
    );

    let expanded = quote! {
        impl #impl_generics ::nebula_action::Action for #struct_name #ty_generics #where_clause {
            fn metadata(&self) -> &::nebula_action::metadata::ActionMetadata {
                use ::std::sync::OnceLock;

                static METADATA: OnceLock<::nebula_action::metadata::ActionMetadata> = OnceLock::new();
                METADATA.get_or_init(|| {
                    use ::nebula_action::metadata::ActionMetadata;
                    #metadata_init
                })
            }
        }
    };

    Ok(expanded.into())
}

fn validate_field_attrs(fields: &Fields) -> syn::Result<()> {
    let mut config_count = 0usize;
    for field in fields {
        let action_attrs = attrs::parse_attrs(&field.attrs, "action")?;
        if action_attrs.has_flag("config") {
            config_count += 1;
        }
    }

    if config_count > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "only one field can be marked with #[action(config)]",
        ));
    }

    Ok(())
}

fn parse_version(version: &str) -> syn::Result<(u32, u32)> {
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

fn parse_action_type(action_type: &str) -> syn::Result<TokenStream2> {
    let variant = match action_type.to_lowercase().as_str() {
        "process" => quote!(::nebula_action::metadata::ActionType::Process),
        "stateful" => quote!(::nebula_action::metadata::ActionType::Stateful),
        "trigger" => quote!(::nebula_action::metadata::ActionType::Trigger),
        "streaming" => quote!(::nebula_action::metadata::ActionType::Streaming),
        "transactional" => quote!(::nebula_action::metadata::ActionType::Transactional),
        "interactive" => quote!(::nebula_action::metadata::ActionType::Interactive),
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "Unknown action type: {}. Expected one of: process, stateful, trigger, streaming, transactional, interactive",
                    action_type
                ),
            ));
        }
    };
    Ok(variant)
}

fn parse_isolation_level(isolation: &str) -> syn::Result<TokenStream2> {
    let level = match isolation.to_lowercase().as_str() {
        "none" => quote!(::nebula_action::capability::IsolationLevel::None),
        "sandbox" => quote!(::nebula_action::capability::IsolationLevel::Sandbox),
        "process" => quote!(::nebula_action::capability::IsolationLevel::Process),
        "vm" => quote!(::nebula_action::capability::IsolationLevel::Vm),
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "Unknown isolation level: {}. Expected one of: none, sandbox, process, vm",
                    isolation
                ),
            ));
        }
    };
    Ok(level)
}

fn generate_metadata_init(
    key: &str,
    name: &str,
    description: &str,
    version_major: u32,
    version_minor: u32,
    action_type: TokenStream2,
    isolation_level: TokenStream2,
    credential: Option<&str>,
) -> TokenStream2 {
    let credential_part = credential
        .map(|c| quote!(.with_credential(#c)))
        .unwrap_or_default();

    quote! {
        ActionMetadata::new(#key, #name, #description)
            .with_version(#version_major, #version_minor)
            .with_action_type(#action_type)
            .with_isolation(#isolation_level)
            #credential_part
    }
}
