//! Owned serde authoring: the macro emits only the requested codec directions.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, GenericParam, Ident, Meta, Token, ext::IdentExt, punctuated::Punctuated,
};

#[derive(Clone, Copy)]
enum Direction {
    Input,
    Output,
    Both,
}

pub(crate) fn expand(options: TokenStream, mut input: DeriveInput) -> syn::Result<TokenStream> {
    let direction = syn::parse2::<Ident>(options)?;
    let direction = match &*direction.to_string() {
        "input" => Direction::Input,
        "output" => Direction::Output,
        "both" => Direction::Both,
        _ => {
            return Err(syn::Error::new_spanned(
                direction,
                "expected schema_type(input), schema_type(output), or schema_type(both)",
            ));
        },
    };
    // Owned derives must not coexist with a separately authored serde projection.
    for attr in &input.attrs {
        if attr.path().is_ident("derive") {
            for path in
                attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)?
            {
                if path.segments.last().is_some_and(|part| {
                    matches!(
                        &*part.ident.to_string(),
                        "Serialize" | "Deserialize" | "Schema" | "PropertyType"
                    )
                }) {
                    return Err(syn::Error::new_spanned(
                        path,
                        "schema_type owns Schema, PropertyType, and the directional serde derives; remove the separate derive",
                    ));
                }
            }
        }
        if attr.path().is_ident("serde") {
            for option in attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)? {
                if option.path().is_ident("crate") {
                    return Err(syn::Error::new_spanned(
                        option,
                        "schema_type owns the serde crate path",
                    ));
                }
            }
        }
    }
    let owner = input
        .ident
        .unraw()
        .to_string()
        .bytes()
        .fold(String::new(), |mut owner, byte| {
            use std::fmt::Write as _;
            // Writing into a `String` cannot fail.
            let _ = write!(owner, "{byte:02x}");
            owner
        });
    let mut defaults = TokenStream::new();
    let fields = match &mut input.data {
        Data::Struct(data) => data.fields.iter_mut().collect::<Vec<_>>(),
        Data::Enum(data) => data
            .variants
            .iter_mut()
            .flat_map(|variant| variant.fields.iter_mut())
            .collect(),
        Data::Union(_) => Vec::new(),
    };
    for (ordinal, field) in fields.into_iter().enumerate() {
        for attr in field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("serde"))
        {
            for option in attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)? {
                if option.path().is_ident("default") {
                    return Err(syn::Error::new_spanned(
                        option,
                        "schema_type owns literal default factories; use property(input(default = literal)) instead of serde default",
                    ));
                }
            }
        }
        let attributes = crate::attrs::FieldAttrs::from_attrs(&field.attrs)?;
        if attributes.emit_as.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "emit_as cannot prove serde fidelity; use serde rename(serialize = ..., deserialize = ...)",
            ));
        }
        if let Some(value) = crate::derive_property_type::literal_default(field, &attributes)? {
            let name = field.ident.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &field,
                    "newtype literal defaults require a reviewed adapter",
                )
            })?;
            let factory = format_ident!(
                "__nebula_schema_{}_{}_{}_default",
                owner,
                name.to_string().trim_start_matches("r#"),
                ordinal,
            );
            let path = syn::LitStr::new(&factory.to_string(), name.span());
            let ty = &field.ty;
            if !matches!(direction, Direction::Output) {
                let declaration = match attributes.default {
                    Some(crate::attrs::DefaultLit::Null) => quote! {
                        fn #factory<__NebulaDefault>() -> ::core::option::Option<__NebulaDefault> { #value }
                    },
                    Some(crate::attrs::DefaultLit::EmptyArray) => {
                        let kind = crate::type_infer::classify(ty);
                        let container = if kind.is_optional() {
                            quote!(::core::option::Option<::std::vec::Vec<__NebulaDefault>>)
                        } else {
                            quote!(::std::vec::Vec<__NebulaDefault>)
                        };
                        quote! { fn #factory<__NebulaDefault>() -> #container { #value } }
                    },
                    _ => quote! { fn #factory() -> #ty { #value } },
                };
                defaults.extend(declaration);
                field
                    .attrs
                    .push(syn::parse_quote!(#[serde(default = #path)]));
            }
        }
        if attributes.skip {
            let projection = crate::codec_attrs::CodecAttrs::parse(&field.attrs)?;
            if !projection.skip_input || !projection.skip_output {
                field.attrs.push(syn::parse_quote!(#[serde(skip)]));
            }
        }
    }
    let serde = nebula_macro_support::paths::resolve_generated_crate_paths(quote!(
        ::nebula_schema::__private::serde
    ));
    let serde_name = syn::LitStr::new(&serde.to_string().replace(' ', ""), input.ident.span());
    let derive_input = !matches!(direction, Direction::Output);
    let derive_output = !matches!(direction, Direction::Input);
    let mut derives = Vec::new();
    if derive_input {
        derives.push(quote!(#serde::Deserialize));
    }
    if derive_output {
        derives.push(quote!(#serde::Serialize));
    }
    let has_schema = crate::derive_property_type::graph_has_schema(&input);
    let mut codecs = TokenStream::new();
    if derive_input {
        codecs.extend(codec_impl(&input, true)?);
    }
    if derive_output {
        codecs.extend(codec_impl(&input, false)?);
    }
    let property_type = crate::derive_property_type::expand_owned(&input)?;
    let select_options = consume_owned_helpers(&mut input)?;
    Ok(quote! {
        #[derive(#(#derives),*)]
        #[serde(crate = #serde_name)]
        #input
        #defaults
        #property_type
        #select_options
        #has_schema
        #codecs
    })
}

/// The attribute owner consumes its helpers after producing graph and catalog
/// implementations. Serde alone does not register schema helper attributes.
fn consume_owned_helpers(input: &mut DeriveInput) -> syn::Result<TokenStream> {
    let mut select_options = TokenStream::new();
    let mut attributes = Vec::new();
    for attribute in &input.attrs {
        if attribute.path().is_ident("derive") {
            let paths =
                attribute.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)?;
            let mut retained = Vec::new();
            for path in paths {
                if path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "EnumSelect")
                {
                    select_options.extend(crate::derive_enum::expand(input.clone())?);
                } else {
                    retained.push(path);
                }
            }
            if !retained.is_empty() {
                attributes.push(syn::parse_quote!(#[derive(#(#retained),*)]));
            }
        } else if !schema_helper(attribute) {
            attributes.push(attribute.clone());
        }
    }
    input.attrs = attributes;
    match &mut input.data {
        Data::Struct(data) => {
            for field in &mut data.fields {
                field.attrs.retain(|attribute| !schema_helper(attribute));
            }
        },
        Data::Enum(data) => {
            for variant in &mut data.variants {
                variant.attrs.retain(|attribute| !schema_helper(attribute));
                for field in &mut variant.fields {
                    field.attrs.retain(|attribute| !schema_helper(attribute));
                }
            }
        },
        Data::Union(_) => {},
    }
    Ok(select_options)
}

fn schema_helper(attribute: &syn::Attribute) -> bool {
    ["schema", "property", "field", "validate"]
        .iter()
        .any(|name| attribute.path().is_ident(name))
}

fn codec_impl(input: &DeriveInput, inbound: bool) -> syn::Result<TokenStream> {
    let schema = crate::crate_path();
    let name = &input.ident;
    let trait_name = if inbound {
        quote!(#schema::InputCodec)
    } else {
        quote!(#schema::OutputCodec)
    };
    let serde_bound = if inbound {
        quote!(#schema::__private::serde::de::DeserializeOwned)
    } else {
        quote!(#schema::__private::serde::Serialize)
    };
    let method = if inbound {
        quote!(input_definition)
    } else {
        quote!(output_definition)
    };
    let direction = if inbound {
        quote!(#schema::SchemaDirection::Input)
    } else {
        quote!(#schema::SchemaDirection::Output)
    };
    let mut generics = input.generics.clone();
    for parameter in &mut generics.params {
        if let GenericParam::Type(parameter) = parameter {
            parameter.bounds.push(syn::parse_quote!(#trait_name));
        }
    }
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(Self: #serde_bound));
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(Self: 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let fields = match &input.data {
        Data::Struct(data) => data.fields.iter().collect::<Vec<_>>(),
        Data::Enum(data) => {
            let mut fields = Vec::new();
            for variant in &data.variants {
                let projection = crate::codec_attrs::CodecAttrs::parse(&variant.attrs)?;
                let skipped = if inbound {
                    projection.skip_input
                } else {
                    projection.skip_output
                };
                if !skipped {
                    fields.extend(variant.fields.iter());
                }
            }
            fields
        },
        Data::Union(_) => Vec::new(),
    };
    let mut types = Vec::new();
    for field in fields {
        let attrs = crate::codec_attrs::CodecAttrs::parse(&field.attrs)?;
        let skipped = if inbound {
            attrs.skip_input
        } else {
            attrs.skip_output
        };
        if !skipped {
            types.push(&field.ty);
        }
    }
    let child_witnesses = if types.is_empty() {
        TokenStream::new()
    } else {
        quote! {
            fn __nebula_require_codec<__NebulaCodec: #trait_name>() {}
            #(__nebula_require_codec::<#types>();)*
        }
    };
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #trait_name for #name #type_generics #where_clause {
            fn #method() -> ::core::result::Result<#schema::AdmittedSchemaGraph, #schema::ValidationReport> {
                // Every included field needs codec provenance, even when optional or
                // in an inactive variant. Serde alone proves no schema agreement.
                #child_witnesses
                <Self as #schema::PropertyType>::definition(#direction)
            }
        }
    })
}
