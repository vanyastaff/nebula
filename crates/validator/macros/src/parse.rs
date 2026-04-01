//! Parse phase: converts `DeriveInput` into the [`ValidatorInput`] IR.
//!
//! All attribute parsing logic lives here. The parser reads `#[validator(...)]`
//! and `#[validate(...)]` attributes and produces a structured IR that the
//! emit phase can generate code from without touching `syn` types.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Type};

use nebula_macro_support::validation_codegen::{
    is_option_type, parse_number_lit, parse_usize, value_token,
};
use nebula_macro_support::{attrs, diag};

use crate::model::{
    ContainerAttrs, EachRules, FieldDef, Rule, StringFactoryKind, StringFormat, ValidatorInput,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse a `DeriveInput` into the validator IR.
///
/// # Errors
///
/// Returns a `syn::Error` when attributes are malformed, types are
/// incompatible with the requested rule, or required sub-attributes are
/// missing.
pub fn parse(input: &DeriveInput) -> syn::Result<ValidatorInput> {
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new(
                    input.ident.span(),
                    "Validator derive requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Validator derive can only be used on structs",
            ));
        }
    };

    let validator_attrs = attrs::parse_attrs(&input.attrs, "validator")?;
    let container = ContainerAttrs {
        message: validator_attrs
            .get_string("message")
            .unwrap_or_else(|| "validation failed".to_string()),
    };

    let mut field_defs = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(ident) = field.ident.clone() else {
            continue;
        };
        let validate_attrs = attrs::parse_attrs(&field.attrs, "validate")?;
        let is_option = is_option_type(&field.ty);
        let inner_ty = option_inner_type(&field.ty)
            .cloned()
            .unwrap_or_else(|| field.ty.clone());
        let message = validate_attrs.get_string("message");

        let rules = parse_field_rules(&validate_attrs, &field.ty, &inner_ty)?;
        let each_rules = parse_each_rules(&validate_attrs, &field.ty, &inner_ty)?;

        field_defs.push(FieldDef {
            ident,
            ty: field.ty.clone(),
            is_option,
            inner_ty,
            message,
            rules,
            each_rules,
        });
    }

    Ok(ValidatorInput {
        ident: input.ident.clone(),
        generics: input.generics.clone(),
        container,
        fields: field_defs,
    })
}

// ---------------------------------------------------------------------------
// Field-level rule parsing
// ---------------------------------------------------------------------------

/// Parse all `#[validate(...)]` attributes on a field into a list of [`Rule`]s.
fn parse_field_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Vec<Rule>> {
    let is_option = is_option_type(original_ty);
    let is_string = is_string_type(inner_ty);
    let is_bool = is_bool_type(inner_ty);
    let is_vec = vec_inner_type(inner_ty).is_some();

    let mut rules = Vec::new();

    // required — only meaningful for Option fields
    if attrs.has_flag("required") && is_option {
        rules.push(Rule::Required);
    }

    // min_length / max_length / exact_length
    if let Some(n) = parse_usize(attrs, "min_length")? {
        rules.push(Rule::MinLength(n));
    }
    if let Some(n) = parse_usize(attrs, "max_length")? {
        rules.push(Rule::MaxLength(n));
    }
    if let Some(n) = parse_usize(attrs, "exact_length")? {
        rules.push(Rule::ExactLength(n));
    }

    // length_range(min = N, max = M)
    if let Some((min, max)) = parse_min_max_list(attrs, "length_range")? {
        require_string_type(original_ty, is_string, "length_range(...)")?;
        rules.push(Rule::LengthRange { min, max });
    }

    // min / max (numeric)
    if let Some(ts) = parse_number_lit(attrs, "min")? {
        rules.push(Rule::Min(ts));
    }
    if let Some(ts) = parse_number_lit(attrs, "max")? {
        rules.push(Rule::Max(ts));
    }

    // min_size / max_size / exact_size
    if let Some(n) = parse_usize(attrs, "min_size")? {
        require_vec_type(original_ty, is_vec, "min_size")?;
        rules.push(Rule::MinSize(n));
    }
    if let Some(n) = parse_usize(attrs, "max_size")? {
        require_vec_type(original_ty, is_vec, "max_size")?;
        rules.push(Rule::MaxSize(n));
    }
    if let Some(n) = parse_usize(attrs, "exact_size")? {
        require_vec_type(original_ty, is_vec, "exact_size")?;
        rules.push(Rule::ExactSize(n));
    }

    // not_empty_collection
    if attrs.has_flag("not_empty_collection") {
        require_vec_type(original_ty, is_vec, "not_empty_collection")?;
        rules.push(Rule::NotEmptyCollection);
    }

    // size_range(min = N, max = M)
    if let Some((min, max)) = parse_min_max_list(attrs, "size_range")? {
        require_vec_type(original_ty, is_vec, "size_range(...)")?;
        rules.push(Rule::SizeRange { min, max });
    }

    // String format flags (not_empty, email, url, etc.)
    for (flag, format) in string_format_flags() {
        if attrs.has_flag(flag) {
            require_string_type(original_ty, is_string, flag)?;
            rules.push(Rule::StringFormat(format));
        }
    }

    // String factory keys (contains, starts_with, ends_with)
    for (key, kind) in string_factory_keys() {
        if let Some(arg) = attrs.get_string(key) {
            require_string_type(original_ty, is_string, &format!("{key} = ..."))?;
            rules.push(Rule::StringFactory { kind, arg });
        }
    }

    // is_true / is_false
    if attrs.has_flag("is_true") {
        require_bool_type(original_ty, is_bool, "is_true")?;
        rules.push(Rule::IsTrue);
    }
    if attrs.has_flag("is_false") {
        require_bool_type(original_ty, is_bool, "is_false")?;
        rules.push(Rule::IsFalse);
    }

    // regex = "pattern"
    if let Some(pattern) = attrs.get_string("regex") {
        require_string_type(original_ty, is_string, "regex = ...")?;
        rules.push(Rule::Regex(pattern));
    }

    // nested
    if attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom = expr
    if let Some(expr) = parse_custom_validator_expr(attrs)? {
        rules.push(Rule::Custom(expr));
    }

    Ok(rules)
}

// ---------------------------------------------------------------------------
// Each-element rule parsing
// ---------------------------------------------------------------------------

/// Parse `each(...)` sub-attributes into [`EachRules`].
fn parse_each_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Option<EachRules>> {
    let Some(each_attrs) = parse_each_args(attrs)? else {
        return Ok(None);
    };

    let element_source_type = option_inner_type(original_ty).unwrap_or(inner_ty);
    let element_ty = vec_inner_type(element_source_type).ok_or_else(|| {
        syn::Error::new_spanned(
            original_ty,
            "`each(...)` is supported for `Vec<T>` and `Option<Vec<T>>` fields",
        )
    })?;

    let each_element_is_string = is_string_type(element_ty);
    let mut rules = Vec::new();

    // min_length / max_length / exact_length
    if let Some(n) = parse_usize(&each_attrs, "min_length")? {
        rules.push(Rule::MinLength(n));
    }
    if let Some(n) = parse_usize(&each_attrs, "max_length")? {
        rules.push(Rule::MaxLength(n));
    }
    if let Some(n) = parse_usize(&each_attrs, "exact_length")? {
        rules.push(Rule::ExactLength(n));
    }

    // min / max (numeric)
    if let Some(ts) = parse_number_lit(&each_attrs, "min")? {
        rules.push(Rule::Min(ts));
    }
    if let Some(ts) = parse_number_lit(&each_attrs, "max")? {
        rules.push(Rule::Max(ts));
    }

    // String format flags
    for (flag, format) in string_format_flags() {
        if each_attrs.has_flag(flag) {
            if !each_element_is_string {
                return Err(syn::Error::new_spanned(
                    original_ty,
                    format!("`each({flag})` requires `Vec<String>` or `Option<Vec<String>>`"),
                ));
            }
            rules.push(Rule::StringFormat(format));
        }
    }

    // String factory keys
    for (key, kind) in string_factory_keys() {
        if let Some(arg) = each_attrs.get_string(key) {
            if !each_element_is_string {
                return Err(syn::Error::new_spanned(
                    original_ty,
                    format!(
                        "`each({key} = ...)` requires `Vec<String>` or `Option<Vec<String>>`"
                    ),
                ));
            }
            rules.push(Rule::StringFactory { kind, arg });
        }
    }

    // regex
    if let Some(pattern) = each_attrs.get_string("regex") {
        if !each_element_is_string {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`each(regex = ...)` requires `Vec<String>` or `Option<Vec<String>>`",
            ));
        }
        rules.push(Rule::Regex(pattern));
    }

    // nested
    if each_attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom
    if let Some(expr) = parse_custom_validator_expr(&each_attrs)? {
        rules.push(Rule::Custom(expr));
    }

    Ok(Some(EachRules { rules }))
}

// ---------------------------------------------------------------------------
// Type-checking helpers
// ---------------------------------------------------------------------------

/// Return an error if the field type is not `String` or `Option<String>`.
fn require_string_type(ty: &Type, is_string: bool, attr_name: &str) -> syn::Result<()> {
    if !is_string {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `String` or `Option<String>` fields"),
        ));
    }
    Ok(())
}

/// Return an error if the field type is not `Vec<T>` or `Option<Vec<T>>`.
fn require_vec_type(ty: &Type, is_vec: bool, attr_name: &str) -> syn::Result<()> {
    if !is_vec {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `Vec<T>` or `Option<Vec<T>>` fields"),
        ));
    }
    Ok(())
}

/// Return an error if the field type is not `bool` or `Option<bool>`.
fn require_bool_type(ty: &Type, is_bool: bool, attr_name: &str) -> syn::Result<()> {
    if !is_bool {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `bool` or `Option<bool>` fields"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mapping tables
// ---------------------------------------------------------------------------

/// Maps attribute flag names to [`StringFormat`] enum variants.
fn string_format_flags() -> Vec<(&'static str, StringFormat)> {
    vec![
        ("not_empty", StringFormat::NotEmpty),
        ("alphanumeric", StringFormat::Alphanumeric),
        ("alphabetic", StringFormat::Alphabetic),
        ("numeric", StringFormat::Numeric),
        ("lowercase", StringFormat::Lowercase),
        ("uppercase", StringFormat::Uppercase),
        ("email", StringFormat::Email),
        ("url", StringFormat::Url),
        ("ipv4", StringFormat::Ipv4),
        ("ipv6", StringFormat::Ipv6),
        ("ip_addr", StringFormat::IpAddr),
        ("hostname", StringFormat::Hostname),
        ("uuid", StringFormat::Uuid),
        ("date", StringFormat::Date),
        ("date_time", StringFormat::DateTime),
        ("time", StringFormat::Time),
    ]
}

/// Maps attribute key names to [`StringFactoryKind`] enum variants.
fn string_factory_keys() -> Vec<(&'static str, StringFactoryKind)> {
    vec![
        ("contains", StringFactoryKind::Contains),
        ("starts_with", StringFactoryKind::StartsWith),
        ("ends_with", StringFactoryKind::EndsWith),
    ]
}

// ---------------------------------------------------------------------------
// Helpers ported from old validator.rs
// ---------------------------------------------------------------------------

/// Extract the inner type from `Option<T>`.
fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}

/// Extract the inner type from `Vec<T>`.
fn vec_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}

/// Check if a type is `String`.
fn is_string_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "String")
        .unwrap_or(false)
}

/// Check if a type is `bool`.
fn is_bool_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "bool")
        .unwrap_or(false)
}

/// Parse `custom = expr` from attribute arguments.
fn parse_custom_validator_expr(args: &attrs::AttrArgs) -> syn::Result<Option<TokenStream2>> {
    let Some(value) = args.get_value("custom") else {
        return Ok(None);
    };

    let expr = match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid custom validator: {e}")))?;
            quote!(#parsed)
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`custom` must be a function path or string path",
            ));
        }
    };

    Ok(Some(expr))
}

/// Parse a `key(min = N, max = M)` list attribute into a `(min, max)` pair.
fn parse_min_max_list(
    validate_attrs: &attrs::AttrArgs,
    key: &str,
) -> syn::Result<Option<(usize, usize)>> {
    let Some(values) = validate_attrs.get_list_values(key) else {
        return Ok(None);
    };

    let mut min: Option<usize> = None;
    let mut max: Option<usize> = None;

    for value in values {
        let item = parse_list_item_to_attr_item(value)?;
        let attrs::AttrItem::KeyValue {
            key: entry_key,
            value,
        } = item
        else {
            return Err(diag::error_spanned(
                &value_token(value),
                format!("`{key}` expects key-value entries like `{key}(min = 1, max = 10)`"),
            ));
        };

        let parsed = match value {
            attrs::AttrValue::Lit(syn::Lit::Int(int)) => {
                int.base10_parse::<usize>().map_err(|_| {
                    diag::error_spanned(&int, format!("`{key}` bounds must be positive integers"))
                })?
            }
            other => {
                return Err(diag::error_spanned(
                    &value_token(&other),
                    format!("`{key}` bounds must be integer literals"),
                ));
            }
        };

        if entry_key == "min" {
            min = Some(parsed);
        } else if entry_key == "max" {
            max = Some(parsed);
        } else {
            return Err(syn::Error::new_spanned(
                entry_key,
                format!("`{key}` only supports `min` and `max` keys"),
            ));
        }
    }

    let min = min.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires both `min` and `max`"),
        )
    })?;
    let max = max.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires both `min` and `max`"),
        )
    })?;

    if min > max {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires `min <= max`"),
        ));
    }

    Ok(Some((min, max)))
}

/// Parse `each(...)` sub-attributes into an `AttrArgs`.
fn parse_each_args(validate_attrs: &attrs::AttrArgs) -> syn::Result<Option<attrs::AttrArgs>> {
    let Some(values) = validate_attrs.get_list_values("each") else {
        return Ok(None);
    };

    let mut items = Vec::with_capacity(values.len());
    for value in values {
        items.push(parse_list_item_to_attr_item(value)?);
    }

    Ok(Some(attrs::AttrArgs { items }))
}

/// Convert a list item value into an `AttrItem`.
fn parse_list_item_to_attr_item(item: &attrs::AttrValue) -> syn::Result<attrs::AttrItem> {
    match item {
        attrs::AttrValue::Ident(ident) => Ok(attrs::AttrItem::Flag(ident.clone())),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid each(...) entry: {e}")))?;
            parse_expr_to_attr_item(parsed, item)
        }
        attrs::AttrValue::Tokens(tokens) => {
            let parsed = syn::parse2::<syn::Expr>(tokens.clone()).map_err(|e| {
                diag::error_spanned(tokens, format!("invalid each(...) entry: {e}"))
            })?;
            parse_expr_to_attr_item(parsed, item)
        }
        attrs::AttrValue::Lit(other) => Err(diag::error_spanned(
            other,
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

/// Convert a parsed expression into an `AttrItem`.
fn parse_expr_to_attr_item(
    expr: syn::Expr,
    span_source: &attrs::AttrValue,
) -> syn::Result<attrs::AttrItem> {
    match expr {
        syn::Expr::Path(path) => {
            if path.path.segments.len() == 1 && path.path.leading_colon.is_none() {
                Ok(attrs::AttrItem::Flag(
                    path.path
                        .segments
                        .into_iter()
                        .next()
                        .expect("segment")
                        .ident,
                ))
            } else {
                Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) flags must be single identifiers",
                ))
            }
        }
        syn::Expr::Assign(assign) => {
            let syn::Expr::Path(left_path) = *assign.left else {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            };
            if left_path.path.segments.len() != 1 || left_path.path.leading_colon.is_some() {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            }
            let key = left_path
                .path
                .segments
                .into_iter()
                .next()
                .expect("segment")
                .ident;
            let value = expr_to_attr_value(*assign.right);
            Ok(attrs::AttrItem::KeyValue { key, value })
        }
        _ => Err(diag::error_spanned(
            &value_token(span_source),
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

/// Convert an expression into an `AttrValue`.
fn expr_to_attr_value(expr: syn::Expr) -> attrs::AttrValue {
    match expr {
        syn::Expr::Path(path) => {
            if path.path.segments.len() == 1 && path.path.leading_colon.is_none() {
                attrs::AttrValue::Ident(
                    path.path
                        .segments
                        .into_iter()
                        .next()
                        .expect("segment")
                        .ident,
                )
            } else {
                attrs::AttrValue::Tokens(quote!(#path))
            }
        }
        syn::Expr::Lit(lit) => attrs::AttrValue::Lit(lit.lit),
        other => attrs::AttrValue::Tokens(quote!(#other)),
    }
}
