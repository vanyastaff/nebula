//! Implementation of `#[derive(Schema)]`.
//!
//! Generates `impl HasSchema for T { fn schema() -> ValidSchema { ... } }`
//! where the schema is computed once and cached behind a `OnceLock`.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataStruct, DeriveInput, Fields, Ident};

use crate::{
    attrs::{DefaultLit, ParamAttrs, SchemaStructAttrs, ValidateAttrs},
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

    let schema_attrs = SchemaStructAttrs::from_attrs(&input.attrs)?;
    let root_rule_tokens: Vec<TokenStream2> = schema_attrs
        .custom
        .iter()
        .map(|lit| {
            quote! {
                .root_rule(#crate_path::Rule::custom(#lit))
            }
        })
        .collect();

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
                            #( #root_rule_tokens )*
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

    if param.enum_select && param.secret {
        return Err(syn::Error::new_spanned(
            field_name,
            "`#[param(enum_select)]` cannot be combined with `#[param(secret)]`",
        ));
    }
    if param.enum_select && matches!(kind, FieldKind::List(_)) {
        return Err(syn::Error::new_spanned(
            field_name,
            "`#[param(enum_select)]` on `Vec<...>` is not supported yet — build the list field manually or omit `enum_select`",
        ));
    }
    if param.enum_select && !matches!(inner, FieldKind::UserDefined(_)) {
        return Err(syn::Error::new_spanned(
            field_name,
            "`#[param(enum_select)]` only applies to enums (or `Option<Enum>`) that implement `HasSelectOptions` via `#[derive(EnumSelect)]`",
        ));
    }
    if param.enum_select {
        ensure_enum_select_validate_attrs(field_name, validate)?;
    }
    if param.enum_select && param.multiline {
        return Err(syn::Error::new_spanned(
            field_name,
            "`#[param(multiline)]` applies only to string fields, not to `#[param(enum_select)]`",
        ));
    }

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
        FieldKind::UserDefined(ty) if param.enum_select => quote! {
            #crate_path::Field::select(#key).extend_options(
                <#ty as #crate_path::HasSelectOptions>::select_options(),
            )
        },
        FieldKind::UserDefined(ty) => quote! {
            #crate_path::Field::object(#key).add_many(
                <#ty as #crate_path::HasSchema>::schema()
                    .fields()
                    .iter()
                    .cloned(),
            )
        },
        FieldKind::UnsupportedInteger(name) => {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "#[derive(Schema)]: integer type `{name}` is not yet supported \
                     because `serde_json::Number` only round-trips through `i64`/`u64`. \
                     Use a narrower integer type (`i8`..`i64`, `u8`..`u64`) or wrap \
                     the value in a newtype that implements `HasSchema` manually."
                ),
            ));
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
        if param.enum_select {
            match default {
                DefaultLit::Str(s) => {
                    expr = quote! { #expr.default(::serde_json::Value::String(#s.to_owned())) };
                },
                _ => {
                    return Err(syn::Error::new_spanned(
                        field_name,
                        "#[param(default = ..)] on `#[param(enum_select)]` fields expects a string literal matching the wire JSON for one variant (for example `\"get\"` for `HttpMethod::Get`).",
                    ));
                },
            }
        } else {
            let default_tokens = default_lit_tokens(default, inner, field_name)?;
            expr = quote! { #expr.default(#default_tokens) };
        }
    }
    if let Some(hint) = &param.hint {
        if param.enum_select {
            return Err(syn::Error::new_spanned(
                field_name,
                "`#[param(hint = ...)]` is not applicable to `#[param(enum_select)]` fields",
            ));
        }
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

/// `#[param(enum_select)]` maps to a `SelectField`; only `#[validate(required)]` is meaningful
/// there.
fn ensure_enum_select_validate_attrs(
    field_name: &Ident,
    validate: &ValidateAttrs,
) -> syn::Result<()> {
    if validate.min_length.is_some()
        || validate.max_length.is_some()
        || validate.pattern.is_some()
        || validate.url
        || validate.email
        || validate.min.is_some()
        || validate.max.is_some()
    {
        return Err(syn::Error::new_spanned(
            field_name,
            "on `#[param(enum_select)]` fields, `#[validate(...)]` supports only `required`; \
             URL, email, pattern, length, and range rules apply to string or number fields",
        ));
    }
    Ok(())
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
        FieldKind::UnsupportedInteger(name) => {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "#[derive(Schema)]: `Vec<{name}>` is not supported because \
                     `{name}` does not round-trip through `serde_json::Number`."
                ),
            ));
        },
    };
    Ok(quote! {
        #crate_path::Field::list(#key).item(#item_expr.into_field())
    })
}

/// Emit the correct `serde_json::Value` constructor for a typed default,
/// rejecting combinations that would ship a wrong-typed default (e.g.
/// `default = "42"` on a `bool` field).
fn default_lit_tokens(
    lit: &DefaultLit,
    inner: &FieldKind,
    field_name: &Ident,
) -> syn::Result<TokenStream2> {
    let mismatch = |expected: &str, got: &str| {
        syn::Error::new_spanned(
            field_name,
            format!("#[param(default = ..)]: field expects {expected}, got {got}"),
        )
    };
    match (inner, lit) {
        // String-ish targets accept only string defaults.
        (FieldKind::String, DefaultLit::Str(s)) => Ok(quote! {
            ::serde_json::Value::String(#s.to_owned())
        }),
        (FieldKind::String, _) => Err(mismatch("a string literal", "non-string literal")),

        // Integer targets accept integer defaults.
        (FieldKind::IntegerNumber, DefaultLit::Int(i)) => Ok(quote! {
            ::serde_json::Value::Number(::serde_json::Number::from(#i))
        }),
        (FieldKind::IntegerNumber, _) => Err(mismatch("an integer literal", "non-integer literal")),

        // Float targets accept both integer (coerced) and float literals.
        (FieldKind::FloatNumber, DefaultLit::Float(f)) => Ok(quote! {
            ::serde_json::Value::Number(
                ::serde_json::Number::from_f64(#f)
                    .expect("derive-provided float default is finite")
            )
        }),
        (FieldKind::FloatNumber, DefaultLit::Int(i)) => Ok(quote! {
            ::serde_json::Value::Number(::serde_json::Number::from(#i))
        }),
        (FieldKind::FloatNumber, _) => Err(mismatch("a numeric literal", "non-numeric literal")),

        (FieldKind::Boolean, DefaultLit::Bool(b)) => Ok(quote! {
            ::serde_json::Value::Bool(#b)
        }),
        (FieldKind::Boolean, _) => Err(mismatch("a bool literal", "non-bool literal")),

        // List / Optional / UserDefined / UnsupportedInteger defaults are
        // explicitly out of scope — container defaults need JSON literals,
        // which aren't expressible through the simple-literal attribute
        // surface. Callers should drop `#[param(default = ..)]` for those.
        _ => Err(syn::Error::new_spanned(
            field_name,
            "#[param(default = ..)] is only supported on String / Number / Boolean fields; \
             Vec / nested object / Option fields cannot carry a literal default",
        )),
    }
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
