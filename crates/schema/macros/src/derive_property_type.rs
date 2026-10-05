//! Type-driven construction of the existing admitted definition graph.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, GenericParam, ext::IdentExt};

use crate::{
    attrs::{FieldAttrs, SchemaStructAttrs, ValidateAttrs},
    codec_attrs::CodecAttrs,
};

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    expand_projection(input, false, true)
}

pub(crate) fn expand_owned(input: &DeriveInput) -> syn::Result<TokenStream> {
    expand_projection(input, true, true)
}

pub(crate) fn expand_legacy_schema(
    input: &DeriveInput,
    validate_secret_input: bool,
) -> syn::Result<TokenStream> {
    expand_projection(input, false, validate_secret_input)
}

fn expand_projection(
    input: &DeriveInput,
    owned_codec: bool,
    validate_secret_input: bool,
) -> syn::Result<TokenStream> {
    let schema = crate::crate_path();
    let name = &input.ident;
    crate::attrs::reject_field_attributes(&input.attrs, "schema containers")?;
    let attrs = CodecAttrs::parse(&input.attrs)?;
    let schema_attrs = SchemaStructAttrs::from_attrs(&input.attrs)?;
    let record_root = matches!(&input.data, Data::Struct(data) if matches!(data.fields, Fields::Named(_)) && !attrs.transparent);
    if !record_root && !schema_attrs.reserved.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "reserved record keys require a record root; use an explicit reviewed graph adapter",
        ));
    }
    let named_conditions = crate::named_conditions::declarations(input, &schema_attrs, &attrs)?;
    let mut reserved_keys = std::collections::HashSet::new();
    for reserved in &schema_attrs.reserved {
        crate::derive_schema::check_field_key(&reserved.value(), reserved.span())?;
        if !reserved_keys.insert(reserved.value()) {
            return Err(syn::Error::new_spanned(reserved, "duplicate reserved key"));
        }
    }
    let unit = matches!(&input.data, Data::Struct(data) if matches!(data.fields, Fields::Unit));
    let body = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Unit => quote!(#schema::__private::serde_json::json!({"kind": "null"})),
            Fields::Named(fields) if !attrs.transparent => record(
                input,
                &fields.named,
                &attrs,
                owned_codec,
                validate_secret_input,
            )?,
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let field = &fields.unnamed[0];
                crate::attrs::reject_field_attributes(&field.attrs, "newtype payloads")?;
                reject_alias_payload_projection(field)?;
                let ty = &field.ty;
                quote!({
                    let mut __alias = <#ty as #schema::PropertyType>::define_schema_type(__builder)?.to_json();
                    __alias["expression"] = #schema::__private::serde_json::json!("allowed");
                    #schema::__private::serde_json::json!({"kind":"alias", "alias":__alias})
                })
            },
            Fields::Named(fields) if attrs.transparent && fields.named.len() == 1 => {
                let field = &fields.named[0];
                crate::attrs::reject_field_attributes(
                    &field.attrs,
                    "transparent newtype payloads",
                )?;
                reject_alias_payload_projection(field)?;
                let ty = &field.ty;
                quote!({
                    let mut __alias = <#ty as #schema::PropertyType>::define_schema_type(__builder)?.to_json();
                    __alias["expression"] = #schema::__private::serde_json::json!("allowed");
                    #schema::__private::serde_json::json!({"kind":"alias", "alias":__alias})
                })
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "PropertyType supports records, unit structs, and single-field newtypes",
                ));
            },
        },
        Data::Enum(data) => {
            let mut variants = Vec::new();
            let mut input_variants = std::collections::HashSet::new();
            let mut output_variants = std::collections::HashSet::new();
            for variant in &data.variants {
                let display = FieldAttrs::from_variant_attrs(&variant.attrs, true)?;
                let projection = CodecAttrs::parse(&variant.attrs)?;
                let (input_key, output_key) = keys(&variant.ident, &projection, &attrs, true)?;
                let skip_input = projection.skip_input;
                let skip_output = projection.skip_output;
                if (!skip_input && !input_variants.insert(input_key.clone()))
                    || (!skip_output && !output_variants.insert(output_key.clone()))
                {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "duplicate directional enum discriminant",
                    ));
                }
                let mut alias_keys = Vec::new();
                for alias in &projection.aliases {
                    crate::derive_schema::check_field_key(alias, variant.ident.span())?;
                    if alias != &input_key && !alias_keys.contains(alias) {
                        if !skip_input && !input_variants.insert(alias.clone()) {
                            return Err(syn::Error::new_spanned(
                                variant,
                                "enum alias collides with another discriminant",
                            ));
                        }
                        alias_keys.push(alias.clone());
                    }
                }
                let label = display.label.as_ref().map(|label| {
                    quote! {
                        __variant["x-label"] = #schema::__private::serde_json::json!(#label);
                    }
                });
                let description = display.description.as_ref().map(|description| quote! {
                    __variant["x-description"] = #schema::__private::serde_json::json!(#description);
                });
                let payload = match &variant.fields {
                    Fields::Unit => quote!(#schema::__private::serde_json::Value::Null),
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        let field = &fields.unnamed[0];
                        crate::attrs::reject_field_attributes(
                            &field.attrs,
                            "enum newtype payloads",
                        )?;
                        reject_alias_payload_projection(field)?;
                        let ty = &field.ty;
                        quote!(<#ty as #schema::PropertyType>::define_schema_type(__builder)?.to_json())
                    },
                    Fields::Named(fields) => {
                        let payload_projection = CodecAttrs {
                            deny_unknown: attrs.deny_unknown,
                            ..CodecAttrs::default()
                        };
                        let body = record(
                            input,
                            &fields.named,
                            &payload_projection,
                            owned_codec,
                            validate_secret_input,
                        )?;
                        let anchor = variant.ident.unraw().to_string();
                        quote!(__builder.define_variant::<Self>(#anchor, |__builder| {
                            ::core::result::Result::Ok(#body)
                        })?.to_json())
                    },
                    _ => {
                        return Err(syn::Error::new_spanned(
                            variant,
                            "tuple enum payloads require an explicit reviewed codec adapter",
                        ));
                    },
                };
                variants.push(quote! {
                    if !match __builder.direction() {
                        #schema::SchemaDirection::Input => #skip_input,
                        #schema::SchemaDirection::Output => #skip_output,
                    } {
                        let __key = match __builder.direction() {
                            #schema::SchemaDirection::Input => #input_key,
                            #schema::SchemaDirection::Output => #output_key,
                        };
                        let mut __variant = #schema::__private::serde_json::json!({"key": __key});
                        #label
                        #description
                        if __builder.direction() == #schema::SchemaDirection::Input {
                            #(__selector_aliases.insert(#alias_keys.to_owned(), #schema::__private::serde_json::json!(__key));)*
                        }
                        let __payload = #payload;
                        if !__payload.is_null() { __variant["payload"] = __payload; }
                        __variants.push(__variant);
                    }
                });
            }
            let tagging = if let (Some(tag), Some(content)) = (&attrs.tag, &attrs.content) {
                quote!(#schema::SerdeTagging::Adjacent { tag: #tag.to_owned(), content: #content.to_owned() })
            } else {
                quote!(#schema::SerdeTagging::External)
            };
            quote!({
                let mut __variants: ::std::vec::Vec<#schema::__private::serde_json::Value> = ::std::vec::Vec::new();
                let mut __selector_aliases = #schema::__private::serde_json::Map::new();
                #(#variants)*
                let __tagging = #schema::__private::serde_json::to_value(#tagging)
                    .map_err(|_| #schema::ValidationError::builder("schema.codec.tag_encoding").build())?;
                let mut __body = #schema::__private::serde_json::json!({"kind":"union", "tagging":__tagging, "variants": __variants});
                if !__selector_aliases.is_empty() {
                    __body["selector_normalization"] = #schema::__private::serde_json::json!({"aliases":__selector_aliases});
                }
                __body
            })
        },
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Rust unions have no checked serde schema projection",
            ));
        },
    };
    let root_rules = schema_attrs
        .custom
        .iter()
        .map(|value| {
            quote! {
                #schema::Rule::custom(#value).map_err(|_| {
                    #schema::ValidationError::builder("rule.budget_exceeded").build()
                })?
            }
        })
        .collect::<Vec<_>>();
    if !root_rules.is_empty()
        && !matches!(&input.data, Data::Struct(data) if matches!(data.fields, Fields::Named(_)) && !attrs.transparent)
    {
        return Err(syn::Error::new_spanned(
            input,
            "root rules on non-record PropertyType require an explicit reviewed adapter",
        ));
    }
    let mut generics = input.generics.clone();
    for parameter in &mut generics.params {
        if let GenericParam::Type(parameter) = parameter {
            parameter
                .bounds
                .push(syn::parse_quote!(#schema::PropertyType));
        }
    }
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(Self: 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let root_null = if unit {
        quote!(.map(#schema::SchemaTypeUse::allow_null))
    } else {
        TokenStream::new()
    };
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #schema::PropertyType for #name #type_generics #where_clause {
            fn define_schema_type(__builder: &mut #schema::SchemaTypeBuilder) -> ::core::result::Result<
                #schema::SchemaTypeUse, #schema::ValidationReport,
            > {
                __builder.define::<Self>(|__builder| {
                    #named_conditions
                    let mut __body = #body;
                    let __rules: ::std::vec::Vec<#schema::Rule> = ::std::vec![#(#root_rules),*];
                    if !__rules.is_empty() {
                        __body["intrinsic_rules"] = #schema::__private::serde_json::to_value(__rules)
                            .map_err(|_| #schema::ValidationError::builder("schema.codec.rule_encoding").build())?;
                    }
                    ::core::result::Result::Ok(__body)
                }) #root_null
            }
        }
    })
}

fn reject_alias_payload_projection(field: &Field) -> syn::Result<()> {
    if let Some(attribute) = field
        .attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("serde"))
    {
        return Err(syn::Error::new_spanned(
            attribute,
            "serde projection attributes on newtype payloads require an explicit reviewed graph adapter",
        ));
    }
    Ok(())
}

pub(crate) fn keys(
    name: &syn::Ident,
    own: &CodecAttrs,
    container: &CodecAttrs,
    variant: bool,
) -> syn::Result<(String, String)> {
    let base = name.unraw().to_string();
    let key = |rename: &Option<String>, rule: Option<crate::attrs::RenameRule>| {
        rename.clone().unwrap_or_else(|| {
            rule.map_or_else(
                || base.clone(),
                |rule| {
                    if variant {
                        rule.apply_to_variant(&base)
                    } else {
                        rule.apply_to_field(&base)
                    }
                },
            )
        })
    };
    let input = key(&own.input_name, container.input_rule);
    let output = key(&own.output_name, container.output_rule);
    crate::derive_schema::check_field_key(&input, name.span())?;
    crate::derive_schema::check_field_key(&output, name.span())?;
    Ok((input, output))
}

fn record(
    input: &DeriveInput,
    fields: &syn::punctuated::Punctuated<Field, syn::Token![,]>,
    container: &CodecAttrs,
    owned_codec: bool,
    validate_secret_input: bool,
) -> syn::Result<TokenStream> {
    let schema = crate::crate_path();
    let schema_attrs = SchemaStructAttrs::from_attrs(&input.attrs)?;
    let mut statements = Vec::new();
    let mut input_keys = std::collections::HashSet::new();
    let mut output_keys = std::collections::HashSet::new();
    for field in fields {
        let name = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
        let mut projection = CodecAttrs::parse(&field.attrs)?;
        let field_attrs = FieldAttrs::from_attrs(&field.attrs)?;
        let validate = ValidateAttrs::from_attrs(&field.attrs)?;
        let (input_key, mut output_key) = keys(name, &projection, container, false)?;
        if let Some(key) = &field_attrs.emit_as {
            crate::derive_schema::check_field_key(key, name.span())?;
            output_key.clone_from(key);
        }
        let skip_input = projection.skip_input || field_attrs.skip;
        let skip_output = projection.skip_output || field_attrs.skip;
        if skip_input && skip_output {
            crate::attrs::check_skipped_attributes(&field.attrs)?;
            continue;
        }
        for (key, skip, seen) in [
            (&input_key, skip_input, &mut input_keys),
            (&output_key, skip_output, &mut output_keys),
        ] {
            if !skip
                && (!seen.insert(key.clone())
                    || schema_attrs
                        .reserved
                        .iter()
                        .any(|value| value.value() == *key))
            {
                return Err(syn::Error::new_spanned(
                    field,
                    "duplicate or reserved directional field key",
                ));
            }
        }
        for alias in &projection.aliases {
            crate::derive_schema::check_field_key(alias, name.span())?;
            if schema_attrs
                .reserved
                .iter()
                .any(|value| value.value() == *alias)
            {
                return Err(syn::Error::new_spanned(field, "input alias is reserved"));
            }
        }
        let mut own_aliases = std::collections::HashSet::new();
        projection
            .aliases
            .retain(|alias| alias != &input_key && own_aliases.insert(alias.clone()));
        if !skip_input {
            for alias in &projection.aliases {
                if !input_keys.insert(alias.clone()) {
                    return Err(syn::Error::new_spanned(
                        field,
                        "input alias collides with another field",
                    ));
                }
            }
        }
        let ty = &field.ty;
        let output_present_type = crate::codec_attrs::output_present_type(&projection, ty)?;
        let optional = crate::type_infer::classify(ty).is_optional();
        let input_presence = if validate.required || !(optional || projection.default) {
            "required"
        } else {
            "optional"
        };
        let output_presence = if projection.optional_output {
            "optional"
        } else {
            "required"
        };
        let aliases = &projection.aliases;
        let facets = property_facets(
            field,
            &field_attrs,
            &validate,
            &input_key,
            validate_secret_input,
        )?;
        let omission_check = output_present_type.map(|inner| {
            quote! {
                let _: fn(#ty) -> ::core::option::Option<#inner> = |__value| __value;
            }
        });
        let legacy_nullable = optional.then(|| {
            if output_present_type.is_some() {
                quote!(.map(|__use| match __builder.direction() {
                    #schema::SchemaDirection::Input => __use.allow_null(),
                    #schema::SchemaDirection::Output => __use,
                }))
            } else {
                quote!(.map(#schema::SchemaTypeUse::allow_null))
            }
        });
        let use_site = if field_attrs.secret && !owned_codec {
            quote!(<::std::string::String as #schema::PropertyType>::define_schema_type(__builder) #legacy_nullable ?)
        } else if field_attrs.enum_select && !owned_codec {
            // The legacy select catalog describes a closed string domain without
            // granting the enum a PropertyType or codec implementation.
            quote! {
                __builder.define_variant::<Self>(#input_key, |_| {
                    ::core::result::Result::Ok(#schema::__private::serde_json::json!({"kind":"string"}))
                }) #legacy_nullable ?
            }
        } else if let Some(inner) = output_present_type {
            quote! {
                {
                    match __builder.direction() {
                        #schema::SchemaDirection::Input => <#ty as #schema::PropertyType>::define_schema_type(__builder)?,
                        #schema::SchemaDirection::Output => <#inner as #schema::PropertyType>::define_schema_type(__builder)?,
                    }
                }
            }
        } else {
            quote!(<#ty as #schema::PropertyType>::define_schema_type(__builder)?)
        };
        statements.push(quote! {
            if !match __builder.direction() {
                #schema::SchemaDirection::Input => #skip_input,
                #schema::SchemaDirection::Output => #skip_output,
            } {
                #omission_check
                let __use = #use_site;
                let mut __property = __use.to_json();
                #facets
                match __builder.direction() {
                    #schema::SchemaDirection::Input => {
                        __property["key"] = #schema::__private::serde_json::json!(#input_key);
                        __property["presence"] = #schema::__private::serde_json::json!(#input_presence);
                        __property["aliases"] = #schema::__private::serde_json::json!({"read":[#(#aliases),*]});
                    },
                    #schema::SchemaDirection::Output => {
                        __property["key"] = #schema::__private::serde_json::json!(#output_key);
                        __property["presence"] = #schema::__private::serde_json::json!(#output_presence);
                    },
                }
                if __builder.direction() == #schema::SchemaDirection::Output {
                    if let ::core::option::Option::Some(__object) = __property.as_object_mut() {
                        __object.remove("input_default");
                        __object.remove("transformers");
                        __object.remove("aliases");
                        __object.insert("expression".to_owned(), #schema::__private::serde_json::json!("forbidden"));
                    }
                }
                __properties.push(__property);
            }
        });
    }
    let additional_input = if container.deny_unknown {
        "closed"
    } else {
        "open"
    };
    Ok(quote!({
        let mut __properties: ::std::vec::Vec<#schema::__private::serde_json::Value> = ::std::vec::Vec::new();
        #(#statements)*
        let __additional = match __builder.direction() {
            #schema::SchemaDirection::Input => #additional_input,
            #schema::SchemaDirection::Output => "closed",
        };
        #schema::__private::serde_json::json!({"kind":"record", "properties":__properties, "additional_properties":__additional})
    }))
}

fn property_facets(
    field: &Field,
    attributes: &FieldAttrs,
    validation: &ValidateAttrs,
    key: &str,
    validate_secret_input: bool,
) -> syn::Result<TokenStream> {
    let schema = crate::crate_path();
    let kind = crate::type_infer::classify(&field.ty);
    let has_facets = field.attrs.iter().any(|attr| {
        ["property", "field", "validate"]
            .iter()
            .any(|name| attr.path().is_ident(name))
    });
    if !has_facets {
        return Ok(TokenStream::new());
    }
    let name = field
        .ident
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
    let declaration = crate::derive_schema::build_graph_field_expr(
        crate::derive_schema::FieldContext {
            name,
            ty: &field.ty,
            key,
        },
        &kind,
        attributes,
        validation,
        &schema,
        validate_secret_input,
    )?;
    let field_type = &field.ty;
    let projected_default = match if attributes.enum_select {
        None
    } else {
        literal_default(field, attributes)?
    } {
        Some(value)
            if matches!(
                attributes.default,
                Some(crate::attrs::DefaultLit::Null | crate::attrs::DefaultLit::EmptyArray)
            ) =>
        {
            let json = if matches!(attributes.default, Some(crate::attrs::DefaultLit::Null)) {
                quote!(#schema::__private::serde_json::Value::Null)
            } else {
                quote!(#schema::__private::serde_json::Value::Array(::std::vec::Vec::new()))
            };
            quote! {
                if __builder.direction() == #schema::SchemaDirection::Input {
                    let __default: #field_type = #value;
                    let _ = __default;
                    __property["input_default"] = #json;
                }
            }
        },
        Some(value) => quote! {
            if __builder.direction() == #schema::SchemaDirection::Input {
                let __default: #field_type = #value;
                __property["input_default"] = #schema::__private::serde_json::to_value(__default)
                    .map_err(|_| #schema::ValidationError::builder("schema.codec.default_encoding").build())?;
            }
        },
        None => TokenStream::new(),
    };
    Ok(quote! {
        let __declaration = #declaration;
        __property = __builder.property(__use, __declaration)?;
        #projected_default
    })
}

pub(crate) fn graph_has_schema(input: &DeriveInput) -> TokenStream {
    let schema = crate::crate_path();
    let name = &input.ident;
    let mut generics = input.generics.clone();
    for parameter in &mut generics.params {
        if let GenericParam::Type(parameter) = parameter {
            parameter
                .bounds
                .push(syn::parse_quote!(#schema::PropertyType));
        }
    }
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(Self: 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics #schema::HasSchema for #name #type_generics #where_clause {
            fn schema() -> ::core::result::Result<#schema::ValidSchema, #schema::ValidationReport> {
                #schema::ValidSchema::from_graph(
                    &<Self as #schema::PropertyType>::definition(#schema::SchemaDirection::Input)?
                )
            }
        }
    }
}

/// One actual Rust value drives both serde's factory and graph default evidence.
pub(crate) fn literal_default(
    field: &Field,
    attributes: &FieldAttrs,
) -> syn::Result<Option<TokenStream>> {
    use crate::{attrs::DefaultLit, type_infer::FieldKind};
    let Some(default) = &attributes.default else {
        return Ok(None);
    };
    let kind = crate::type_infer::classify(&field.ty);
    if matches!(default, DefaultLit::Null) {
        if !kind.is_optional() {
            return Err(syn::Error::new_spanned(
                field,
                "null default requires an Option<T> field",
            ));
        }
        return Ok(Some(quote!(::core::option::Option::None)));
    }
    let value = match (kind.inner(), default) {
        (FieldKind::List(_), DefaultLit::EmptyArray) => quote!(::std::vec::Vec::new()),
        (FieldKind::String, DefaultLit::Str(value)) => {
            let literal = syn::LitStr::new(value, proc_macro2::Span::call_site());
            quote!(::std::string::String::from(#literal))
        },
        (FieldKind::Boolean, DefaultLit::Bool(value)) => quote!(#value),
        (FieldKind::IntegerNumber(_), DefaultLit::Int(value)) => {
            let value = syn::parse_str::<syn::Expr>(&value.to_string())?;
            quote!(#value)
        },
        (FieldKind::FloatNumber(_), DefaultLit::Int(value)) => {
            let value = syn::parse_str::<syn::Expr>(&format!("{value}.0"))?;
            quote!(#value)
        },
        (FieldKind::FloatNumber(_), DefaultLit::Float(value)) if value.is_finite() => {
            let mut spelling = value.to_string();
            if !spelling.contains(['.', 'e', 'E']) {
                spelling.push_str(".0");
            }
            let value = syn::parse_str::<syn::Expr>(&spelling)?;
            quote!(#value)
        },
        _ => {
            return Err(syn::Error::new_spanned(
                field,
                "codec defaults require a matching primitive literal, null on Option<T>, or [] on Vec<T>",
            ));
        },
    };
    Ok(Some(if kind.is_optional() {
        quote!(::core::option::Option::Some(#value))
    } else {
        value
    }))
}
