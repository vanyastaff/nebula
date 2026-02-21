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

    let metadata_init =
        generate_metadata_init(&key, &name, &description, version_major, version_minor);

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

fn generate_metadata_init(
    key: &str,
    name: &str,
    description: &str,
    version_major: u32,
    version_minor: u32,
) -> TokenStream2 {
    quote! {
        ActionMetadata::new(#key, #name, #description)
            .with_version(#version_major, #version_minor)
    }
}
