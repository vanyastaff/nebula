//! Implementation of `#[derive(Schema)]`.
//!
//! Generates `impl HasSchema for T { fn schema() -> ValidSchema { ... } }`
//! where the schema is computed once and cached behind a `OnceLock`.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataStruct, DeriveInput, Fields, Ident};

use crate::{
    attrs::{ParamAttrs, ValidateAttrs},
    type_infer::{FieldKind, classify},
};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let crate_path = crate::crate_path();
    let ty_name = &input.ident;
    let generics = &input.generics;
    let (impl_g, ty_g, where_g) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(named),
            ..
        }) => &named.named,
        Data::Struct(_) => {
            return Err(syn::Error::new_spanned(
                ty_name,
                "#[derive(Schema)] only supports structs with named fields",
            ));
        },
        Data::Enum(_) | Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                ty_name,
                "#[derive(Schema)] only supports structs; use #[derive(EnumSelect)] for enums",
            ));
        },
    };

    let mut field_exprs = Vec::with_capacity(fields.len());
    for f in fields {
        let field_name = f
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(f, "anonymous struct field"))?;
        let param = ParamAttrs::from_attrs(&f.attrs)?;
        if param.skip {
            continue;
        }
        let validate = ValidateAttrs::from_attrs(&f.attrs)?;
        let kind = classify(&f.ty);
        let expr = build_field_expr(field_name, &kind, &param, &validate, &crate_path)?;
        field_exprs.push(expr);
    }

    let ty_name_str = ty_name.to_string();
    Ok(quote! {
        impl #impl_g #crate_path::HasSchema for #ty_name #ty_g #where_g {
            fn schema() -> #crate_path::ValidSchema {
                static __CACHE: ::std::sync::OnceLock<#crate_path::ValidSchema> =
                    ::std::sync::OnceLock::new();
                __CACHE
                    .get_or_init(|| {
                        match #crate_path::Schema::builder()
                            #( .add(#field_exprs) )*
                            .build()
                        {
                            ::core::result::Result::Ok(s) => s,
                            ::core::result::Result::Err(report) => ::core::panic!(
                                "#[derive(Schema)] on `{}` produced an invalid schema — \
                                 attribute combinations conflict with a schema-level lint. \
                                 Fix the `#[param(..)]` / `#[validate(..)]` attributes on this type. \
                                 Report: {:?}",
                                #ty_name_str,
                                report,
                            ),
                        }
                    })
                    .clone()
            }
        }
    })
}

/// Build the token-stream expression that produces a `Field` for one struct field.
fn build_field_expr(
    field_name: &Ident,
    kind: &FieldKind,
    param: &ParamAttrs,
    validate: &ValidateAttrs,
    crate_path: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let key = field_name.to_string();
    let optional = kind.is_optional();
    let inner = kind.inner();

    // Pick the constructor by inner kind. `param.secret` forces String → Secret.
    let mut expr = match inner {
        FieldKind::String if param.secret => quote! {
            #crate_path::Field::secret(#key)
        },
        FieldKind::String => quote! {
            #crate_path::Field::string(#key)
        },
        FieldKind::Boolean => quote! {
            #crate_path::Field::boolean(#key)
        },
        FieldKind::IntegerNumber => quote! {
            #crate_path::Field::integer(#key)
        },
        FieldKind::FloatNumber => quote! {
            #crate_path::Field::number(#key)
        },
        FieldKind::List(item_kind) => list_field_expr(field_name, item_kind, crate_path)?,
        FieldKind::Optional(_) => {
            // Cannot nest `Option<Option<T>>`; classify already flattened one layer.
            return Err(syn::Error::new_spanned(
                field_name,
                "nested `Option<Option<..>>` is not supported",
            ));
        },
        FieldKind::UserDefined(ty) => quote! {
            #crate_path::Field::object(#key).add_many(
                <#ty as #crate_path::HasSchema>::schema()
                    .fields()
                    .iter()
                    .cloned(),
            )
        },
    };

    if let Some(label) = &param.label {
        expr = quote! { #expr.label(#label) };
    }
    if let Some(desc) = &param.description {
        expr = quote! { #expr.description(#desc) };
    }
    if let Some(placeholder) = &param.placeholder {
        expr = quote! { #expr.placeholder(#placeholder) };
    }
    if let Some(default) = &param.default {
        expr = quote! {
            #expr.default(::serde_json::Value::String(#default.to_owned()))
        };
    }
    if let Some(hint) = &param.hint {
        let hint_ident = input_hint_ident(hint, field_name)?;
        expr = quote! { #expr.hint(#crate_path::InputHint::#hint_ident) };
    }
    if let Some(group) = &param.group {
        expr = quote! { #expr.group(#group) };
    }
    if param.multiline && matches!(inner, FieldKind::String) && !param.secret {
        expr = quote! { #expr.widget(#crate_path::StringWidget::Multiline) };
    }
    if param.no_expression {
        expr = quote! { #expr.no_expression() };
    }
    if param.expression_required {
        expr = quote! {
            #expr.expression_mode(#crate_path::ExpressionMode::Required)
        };
    }

    // Required: mark when `#[validate(required)]` or the Rust type is not Option.
    if validate.required || !optional {
        expr = quote! { #expr.required() };
    }

    // Length rules apply to String / Secret.
    if let Some(min) = validate.min_length {
        expr = quote! { #expr.min_length(#min) };
    }
    if let Some(max) = validate.max_length {
        expr = quote! { #expr.max_length(#max) };
    }

    // Range rules apply to Number.
    if let Some(min) = validate.min
        && matches!(inner, FieldKind::IntegerNumber | FieldKind::FloatNumber)
    {
        expr = quote! { #expr.min(#min) };
    }
    if let Some(max) = validate.max
        && matches!(inner, FieldKind::IntegerNumber | FieldKind::FloatNumber)
    {
        expr = quote! { #expr.max(#max) };
    }

    if let Some(pattern) = &validate.pattern
        && matches!(inner, FieldKind::String)
    {
        expr = quote! { #expr.pattern(#pattern) };
    }
    if validate.url && matches!(inner, FieldKind::String) {
        expr = quote! { #expr.url() };
    }
    if validate.email && matches!(inner, FieldKind::String) {
        expr = quote! { #expr.email() };
    }

    Ok(quote! { #expr.into_field() })
}

fn list_field_expr(
    field_name: &Ident,
    item_kind: &FieldKind,
    crate_path: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let key = field_name.to_string();
    let item_key = format!("{key}_item");
    let item_expr = match item_kind {
        FieldKind::String => quote! { #crate_path::Field::string(#item_key) },
        FieldKind::Boolean => quote! { #crate_path::Field::boolean(#item_key) },
        FieldKind::IntegerNumber => quote! { #crate_path::Field::integer(#item_key) },
        FieldKind::FloatNumber => quote! { #crate_path::Field::number(#item_key) },
        FieldKind::UserDefined(ty) => quote! {
            #crate_path::Field::object(#item_key).add_many(
                <#ty as #crate_path::HasSchema>::schema()
                    .fields()
                    .iter()
                    .cloned(),
            )
        },
        FieldKind::List(_) | FieldKind::Optional(_) => {
            return Err(syn::Error::new_spanned(
                field_name,
                "nested `Vec<Vec<..>>` or `Vec<Option<..>>` are not supported yet",
            ));
        },
    };
    Ok(quote! {
        #crate_path::Field::list(#key).item(#item_expr.into_field())
    })
}

/// Map `#[param(hint = "...")]` string to the corresponding `InputHint` variant.
fn input_hint_ident(hint: &str, span_source: &Ident) -> syn::Result<Ident> {
    let variant = match hint {
        "text" => "Text",
        "url" => "Url",
        "email" => "Email",
        "password" => "Password",
        "phone" | "tel" => "Phone",
        "ip" => "Ip",
        "regex" => "Regex",
        "markdown" => "Markdown",
        "cron" => "Cron",
        "date" => "Date",
        "date_time" | "datetime" => "DateTime",
        "time" => "Time",
        "color" => "Color",
        "duration" => "Duration",
        "uuid" => "Uuid",
        other => {
            return Err(syn::Error::new(
                span_source.span(),
                format!(
                    "unknown #[param(hint = \"{other}\")]; expected one of: \
                     text, url, email, password, phone, ip, regex, markdown, cron, \
                     date, date_time, time, color, duration, uuid"
                ),
            ));
        },
    };
    Ok(Ident::new(variant, span_source.span()))
}
