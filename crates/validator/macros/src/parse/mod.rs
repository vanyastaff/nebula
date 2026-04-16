//! Parse phase: converts `DeriveInput` into the [`ValidatorInput`] IR.
//!
//! All attribute parsing logic lives here. The parser reads `#[validator(...)]`
//! and `#[validate(...)]` attributes and produces a structured IR that the
//! emit phase can generate code from without touching `syn` types.

#![forbid(unsafe_code)]

use nebula_macro_support::{
    attrs, diag,
    validation_codegen::{is_option_type, value_token},
};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Type, parse::Parser};

use crate::model::{
    ContainerAttrs, FieldDef, Rule, StringFactoryKind, StringFormat, ValidatorInput,
};

mod each;
mod rules;

use each::parse_each_rules;
use rules::parse_field_rules;

/// Validate a regex pattern at macro-time so bad patterns surface as
/// compile errors rather than runtime panics.
///
/// The `spanned` argument gives the diagnostic a span to point at — pass
/// the field's `Type` or any other syn node near the attribute.
pub(super) fn validate_regex_pattern<T: quote::ToTokens>(
    pattern: &str,
    spanned: &T,
) -> syn::Result<()> {
    regex::Regex::new(pattern).map_err(|e| {
        syn::Error::new_spanned(spanned, format!("invalid regex pattern `{pattern}`: {e}"))
    })?;
    Ok(())
}

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
            },
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Validator derive can only be used on structs",
            ));
        },
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
// Type-checking helpers
// ---------------------------------------------------------------------------

/// Return an error if the field type is not `String` or `Option<String>`.
pub(super) fn require_string_type(ty: &Type, is_string: bool, attr_name: &str) -> syn::Result<()> {
    if !is_string {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `String` or `Option<String>` fields"),
        ));
    }
    Ok(())
}

/// Return an error if the field type is not `Vec<T>` or `Option<Vec<T>>`.
pub(super) fn require_vec_type(ty: &Type, is_vec: bool, attr_name: &str) -> syn::Result<()> {
    if !is_vec {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `Vec<T>` or `Option<Vec<T>>` fields"),
        ));
    }
    Ok(())
}

/// Return an error if the field type is not `bool` or `Option<bool>`.
pub(super) fn require_bool_type(ty: &Type, is_bool: bool, attr_name: &str) -> syn::Result<()> {
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

const STRING_FORMAT_FLAGS: [(&str, StringFormat); 16] = [
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
];

const STRING_FACTORY_KEYS: [(&str, StringFactoryKind); 3] = [
    ("contains", StringFactoryKind::Contains),
    ("starts_with", StringFactoryKind::StartsWith),
    ("ends_with", StringFactoryKind::EndsWith),
];

/// Maps attribute flag names to [`StringFormat`] enum variants.
pub(super) fn string_format_flags() -> &'static [(&'static str, StringFormat)] {
    &STRING_FORMAT_FLAGS
}

/// Maps attribute key names to [`StringFactoryKind`] enum variants.
pub(super) fn string_factory_keys() -> &'static [(&'static str, StringFactoryKind)] {
    &STRING_FACTORY_KEYS
}

// ---------------------------------------------------------------------------
// Helpers ported from old validator.rs
// ---------------------------------------------------------------------------

// Type-introspection helpers live in `crate::types` and are used by both
// parse and emit. Re-use them locally for brevity.
use crate::types::{is_bool_type, is_string_type, option_inner_type, vec_inner_type};

/// Parse a validator expression from attribute arguments.
pub(super) fn parse_validator_expr(
    args: &attrs::AttrArgs,
    key: &str,
) -> syn::Result<Option<TokenStream2>> {
    let Some(value) = args.get_value(key) else {
        return Ok(None);
    };

    let expr = match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid `{key}` validator: {e}")))?;
            quote!(#parsed)
        },
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{key}` must be a path, expression, or string expression"),
            ));
        },
    };

    Ok(Some(expr))
}

/// Parse `key(expr1, expr2, ...)` into validator expressions.
pub(super) fn parse_validator_expr_list(
    args: &attrs::AttrArgs,
    key: &str,
) -> syn::Result<Option<Vec<TokenStream2>>> {
    if let Some(values) = args.get_list_values(key) {
        if values.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{key}(...)` requires at least one validator expression"),
            ));
        }

        let mut exprs = Vec::with_capacity(values.len());
        for value in values {
            exprs.extend(parse_validator_expr_values(value, key)?);
        }

        return Ok(Some(exprs));
    }

    let Some(value) = args.get_value(key) else {
        return Ok(None);
    };

    let attrs::AttrValue::Tokens(tokens) = value else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` expects a validator list, e.g. `{key} = [v1, v2]`"),
        ));
    };

    let array = syn::parse2::<syn::ExprArray>(tokens.clone()).map_err(|e| {
        diag::error_spanned(
            tokens,
            format!("`{key}` expects array syntax, e.g. `{key} = [v1, v2]`: {e}"),
        )
    })?;

    if array.elems.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` array must contain at least one validator expression"),
        ));
    }

    let exprs = array.elems.into_iter().map(|expr| quote!(#expr)).collect();
    Ok(Some(exprs))
}

pub(super) fn parse_validator_expr_values(
    value: &attrs::AttrValue,
    key: &str,
) -> syn::Result<Vec<TokenStream2>> {
    let exprs = match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Tokens(tokens) => {
            let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
            if let Ok(list) = parser.parse2(tokens.clone()) {
                let parsed: Vec<TokenStream2> =
                    list.into_iter().map(|expr| quote!(#expr)).collect();
                if !parsed.is_empty() {
                    return Ok(parsed);
                }
            }
            tokens.clone()
        },
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid `{key}` validator: {e}")))?;
            quote!(#parsed)
        },
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{key}` accepts only validator paths or expressions"),
            ));
        },
    };

    Ok(vec![exprs])
}

pub(super) fn parse_call_style_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    value_ty: &Type,
    allows_required: bool,
    is_string: bool,
    is_bool: bool,
    is_vec: bool,
) -> syn::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    for item in &attrs.items {
        let attrs::AttrItem::List { key, values } = item else {
            continue;
        };

        match key.to_string().as_str() {
            "length" => rules.extend(parse_length_call(
                values,
                original_ty,
                value_ty,
                is_string,
                is_vec,
            )?),
            "range" => rules.extend(parse_range_call(values)?),
            "min" => rules.push(Rule::Min(parse_single_expr_call(values, "min")?)),
            "max" => rules.push(Rule::Max(parse_single_expr_call(values, "max")?)),
            "contains" => {
                require_string_type(original_ty, is_string, "contains(...)")?;
                rules.push(Rule::StringFactory {
                    kind: StringFactoryKind::Contains,
                    arg: parse_single_string_call(values, "contains")?,
                });
            },
            "prefix" => {
                require_string_type(original_ty, is_string, "prefix(...)")?;
                rules.push(Rule::StringFactory {
                    kind: StringFactoryKind::StartsWith,
                    arg: parse_single_string_call(values, "prefix")?,
                });
            },
            "suffix" => {
                require_string_type(original_ty, is_string, "suffix(...)")?;
                rules.push(Rule::StringFactory {
                    kind: StringFactoryKind::EndsWith,
                    arg: parse_single_string_call(values, "suffix")?,
                });
            },
            "regex" => {
                require_string_type(original_ty, is_string, "regex(...)")?;
                let pattern = parse_single_string_call(values, "regex")?;
                validate_regex_pattern(&pattern, original_ty)?;
                rules.push(Rule::Regex(pattern));
            },
            "custom" => rules.push(Rule::Custom(parse_single_expr_call(values, "custom")?)),
            "using" => rules.push(Rule::Using(parse_single_expr_call(values, "using")?)),
            "and" => {
                for value in values {
                    rules.extend(parse_rule_call_value(value, original_ty, value_ty)?);
                }
            },
            "or" => {
                let mut exprs = Vec::with_capacity(values.len());
                for value in values {
                    exprs.push(parse_rule_validator_expr(value, original_ty, value_ty)?);
                }
                rules.push(Rule::Any(exprs));
            },
            "required" if values.is_empty() => {
                if allows_required {
                    rules.push(Rule::Required);
                } else {
                    return Err(syn::Error::new_spanned(
                        original_ty,
                        "`required()` requires `Option<T>` values",
                    ));
                }
            },
            "nested" if values.is_empty() => rules.push(Rule::Nested),
            "email" if values.is_empty() => {
                require_string_type(original_ty, is_string, "email()")?;
                rules.push(Rule::StringFormat(StringFormat::Email));
            },
            "url" if values.is_empty() => {
                require_string_type(original_ty, is_string, "url()")?;
                rules.push(Rule::StringFormat(StringFormat::Url));
            },
            "not_empty" if values.is_empty() => {
                if is_string {
                    rules.push(Rule::StringFormat(StringFormat::NotEmpty));
                } else if is_vec {
                    rules.push(Rule::NotEmptyCollection);
                } else {
                    return Err(syn::Error::new_spanned(
                        original_ty,
                        "`not_empty()` requires String-like or Vec-like fields",
                    ));
                }
            },
            "is_true" if values.is_empty() => {
                require_bool_type(original_ty, is_bool, "is_true()")?;
                rules.push(Rule::IsTrue);
            },
            "is_false" if values.is_empty() => {
                require_bool_type(original_ty, is_bool, "is_false()")?;
                rules.push(Rule::IsFalse);
            },
            _ => {},
        }
    }

    Ok(rules)
}

pub(super) fn parse_length_call(
    values: &[attrs::AttrValue],
    original_ty: &Type,
    value_ty: &Type,
    is_string: bool,
    is_vec: bool,
) -> syn::Result<Vec<Rule>> {
    if !is_string && !is_vec {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`length(...)` requires String-like or Vec-like fields",
        ));
    }

    let mut min = None;
    let mut max = None;
    let mut equal = None;

    if values.len() == 1
        && let attrs::AttrValue::Lit(syn::Lit::Int(int)) = &values[0]
    {
        let exact = int
            .base10_parse::<usize>()
            .map_err(|_| diag::error_spanned(int, "`length(...)` requires a positive integer"))?;
        return Ok(vec![if is_string {
            Rule::ExactLength(exact)
        } else {
            Rule::ExactSize(exact)
        }]);
    }

    for value in values {
        let item = parse_list_item_to_attr_item(value)?;
        let attrs::AttrItem::KeyValue { key, value } = item else {
            return Err(diag::error_spanned(
                &value_token(value),
                "`length(...)` expects `length(6)` or key-value entries like `length(min = 1, max = 10)`",
            ));
        };

        let parsed = match value {
            attrs::AttrValue::Lit(syn::Lit::Int(int)) => {
                int.base10_parse::<usize>().map_err(|_| {
                    diag::error_spanned(&int, "`length(...)` bounds must be positive integers")
                })?
            },
            other => {
                return Err(diag::error_spanned(
                    &value_token(&other),
                    "`length(...)` bounds must be integer literals",
                ));
            },
        };

        match key.to_string().as_str() {
            "min" => min = Some(parsed),
            "max" => max = Some(parsed),
            "equal" => equal = Some(parsed),
            _ => {
                return Err(syn::Error::new_spanned(
                    key,
                    "`length(...)` only supports `min`, `max`, and `equal` keys",
                ));
            },
        }
    }

    if equal.is_some() && (min.is_some() || max.is_some()) {
        return Err(syn::Error::new_spanned(
            value_ty,
            "`length(equal = ...)` cannot be combined with `min` or `max`",
        ));
    }

    let mut rules = Vec::new();
    if let Some(exact) = equal {
        rules.push(if is_string {
            Rule::ExactLength(exact)
        } else {
            Rule::ExactSize(exact)
        });
        return Ok(rules);
    }

    match (min, max) {
        (Some(min), Some(max)) => rules.push(if is_string {
            Rule::LengthRange { min, max }
        } else {
            Rule::SizeRange { min, max }
        }),
        (Some(min), None) => rules.push(if is_string {
            Rule::MinLength(min)
        } else {
            Rule::MinSize(min)
        }),
        (None, Some(max)) => rules.push(if is_string {
            Rule::MaxLength(max)
        } else {
            Rule::MaxSize(max)
        }),
        (None, None) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`length(...)` requires either a positional integer or `min`/`max`/`equal`",
            ));
        },
    }

    Ok(rules)
}

pub(super) fn parse_range_call(values: &[attrs::AttrValue]) -> syn::Result<Vec<Rule>> {
    let mut min = None;
    let mut max = None;
    let mut equal = None;

    for value in values {
        let item = parse_list_item_to_attr_item(value)?;
        let attrs::AttrItem::KeyValue { key, value } = item else {
            return Err(diag::error_spanned(
                &value_token(value),
                "`range(...)` expects key-value entries like `range(min = 1, max = 10)`",
            ));
        };

        match key.to_string().as_str() {
            "min" => min = Some(parse_expr_from_attr_value(&value, "range")?),
            "max" => max = Some(parse_expr_from_attr_value(&value, "range")?),
            "equal" => equal = Some(parse_expr_from_attr_value(&value, "range")?),
            _ => {
                return Err(syn::Error::new_spanned(
                    key,
                    "`range(...)` only supports `min`, `max`, and `equal` keys",
                ));
            },
        }
    }

    if equal.is_some() && (min.is_some() || max.is_some()) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`range(equal = ...)` cannot be combined with `min` or `max`",
        ));
    }

    let mut rules = Vec::new();
    if let Some(equal) = equal {
        rules.push(Rule::Min(equal.clone()));
        rules.push(Rule::Max(equal));
        return Ok(rules);
    }
    if let Some(min) = min {
        rules.push(Rule::Min(min));
    }
    if let Some(max) = max {
        rules.push(Rule::Max(max));
    }
    if rules.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`range(...)` requires at least one of `min`, `max`, or `equal`",
        ));
    }

    Ok(rules)
}

pub(super) fn parse_single_expr_call(
    values: &[attrs::AttrValue],
    key: &str,
) -> syn::Result<TokenStream2> {
    if values.len() != 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}(...)` requires exactly one argument"),
        ));
    }

    parse_expr_from_attr_value(&values[0], key)
}

pub(super) fn parse_single_string_call(
    values: &[attrs::AttrValue],
    key: &str,
) -> syn::Result<String> {
    if values.len() != 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}(...)` requires exactly one string argument"),
        ));
    }

    match &values[0] {
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => Ok(s.value()),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}(...)` requires a string literal"),
        )),
    }
}

pub(super) fn parse_expr_from_attr_value(
    value: &attrs::AttrValue,
    _key: &str,
) -> syn::Result<TokenStream2> {
    match value {
        attrs::AttrValue::Ident(ident) => Ok(quote!(#ident)),
        attrs::AttrValue::Tokens(tokens) => Ok(tokens.clone()),
        attrs::AttrValue::Lit(lit) => Ok(quote!(#lit)),
    }
}

pub(super) fn parse_rule_call_value(
    value: &attrs::AttrValue,
    original_ty: &Type,
    value_ty: &Type,
) -> syn::Result<Vec<Rule>> {
    let item = parse_list_item_to_attr_item(value)?;
    match item {
        attrs::AttrItem::Flag(flag) => parse_call_style_rules(
            &attrs::AttrArgs {
                items: vec![attrs::AttrItem::List {
                    key: flag,
                    values: vec![],
                }],
            },
            original_ty,
            value_ty,
            false,
            is_string_type(value_ty),
            is_bool_type(value_ty),
            vec_inner_type(value_ty).is_some(),
        ),
        attrs::AttrItem::List { key, values } => parse_call_style_rules(
            &attrs::AttrArgs {
                items: vec![attrs::AttrItem::List { key, values }],
            },
            original_ty,
            value_ty,
            false,
            is_string_type(value_ty),
            is_bool_type(value_ty),
            vec_inner_type(value_ty).is_some(),
        ),
        attrs::AttrItem::KeyValue { .. } => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`and(...)` only accepts rule calls like `length(6)` or `max(10)`",
        )),
    }
}

pub(super) fn parse_rule_validator_expr(
    value: &attrs::AttrValue,
    original_ty: &Type,
    value_ty: &Type,
) -> syn::Result<TokenStream2> {
    let item = parse_list_item_to_attr_item(value)?;
    match item {
        attrs::AttrItem::Flag(flag) => dsl_item_to_validator_expr(
            &attrs::AttrItem::List {
                key: flag,
                values: vec![],
            },
            original_ty,
            value_ty,
        ),
        item @ attrs::AttrItem::List { .. } => {
            dsl_item_to_validator_expr(&item, original_ty, value_ty)
        },
        attrs::AttrItem::KeyValue { .. } => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`or(...)` only accepts rule calls like `length(6)` or `max(10)`",
        )),
    }
}

pub(super) fn dsl_item_to_validator_expr(
    item: &attrs::AttrItem,
    original_ty: &Type,
    value_ty: &Type,
) -> syn::Result<TokenStream2> {
    let attrs::AttrItem::List { key, values } = item else {
        unreachable!("only list items are converted to validator expressions");
    };

    let is_string = is_string_type(value_ty);
    let is_vec = vec_inner_type(value_ty).is_some();
    let is_bool = is_bool_type(value_ty);
    let element_ty = vec_inner_type(value_ty).unwrap_or(value_ty);

    match key.to_string().as_str() {
        "min" => {
            let expr = parse_single_expr_call(values, "min")?;
            Ok(quote!(::nebula_validator::validators::min(#expr)))
        },
        "max" => {
            let expr = parse_single_expr_call(values, "max")?;
            Ok(quote!(::nebula_validator::validators::max(#expr)))
        },
        "length" => {
            let rules = parse_length_call(values, original_ty, value_ty, is_string, is_vec)?;
            rules_to_validator_expr(&rules, value_ty, element_ty)
        },
        "range" => {
            let rules = parse_range_call(values)?;
            rules_to_validator_expr(&rules, value_ty, element_ty)
        },
        "contains" => {
            let arg = parse_single_string_call(values, "contains")?;
            Ok(quote!(::nebula_validator::validators::contains(#arg)))
        },
        "prefix" => {
            let arg = parse_single_string_call(values, "prefix")?;
            Ok(quote!(::nebula_validator::validators::starts_with(#arg)))
        },
        "suffix" => {
            let arg = parse_single_string_call(values, "suffix")?;
            Ok(quote!(::nebula_validator::validators::ends_with(#arg)))
        },
        "regex" => {
            let arg = parse_single_string_call(values, "regex")?;
            validate_regex_pattern(&arg, value_ty)?;
            Ok(
                quote!(::nebula_validator::validators::matches_regex(#arg).expect("regex validated by derive parser")),
            )
        },
        "email" if values.is_empty() => Ok(quote!(::nebula_validator::validators::email())),
        "url" if values.is_empty() => Ok(quote!(::nebula_validator::validators::url())),
        "not_empty" if values.is_empty() && is_string => {
            Ok(quote!(::nebula_validator::validators::not_empty()))
        },
        "not_empty" if values.is_empty() && is_vec => {
            Ok(quote!(::nebula_validator::validators::not_empty_collection::<#element_ty>()))
        },
        "is_true" if values.is_empty() && is_bool => {
            Ok(quote!(::nebula_validator::validators::is_true()))
        },
        "is_false" if values.is_empty() && is_bool => {
            Ok(quote!(::nebula_validator::validators::is_false()))
        },
        "nested" if values.is_empty() => {
            Ok(quote!(::nebula_validator::combinators::nested_validator::<#value_ty>()))
        },
        "using" => {
            let expr = parse_single_expr_call(values, "using")?;
            Ok(quote!((#expr)))
        },
        "and" => {
            let exprs = values
                .iter()
                .map(|value| parse_rule_validator_expr(value, original_ty, value_ty))
                .collect::<syn::Result<Vec<_>>>()?;
            chain_validator_exprs(exprs, true)
        },
        "or" => {
            let exprs = values
                .iter()
                .map(|value| parse_rule_validator_expr(value, original_ty, value_ty))
                .collect::<syn::Result<Vec<_>>>()?;
            chain_validator_exprs(exprs, false)
        },
        _ => Err(syn::Error::new_spanned(
            key,
            "unsupported rule inside `or(...)`/`and(...)` group",
        )),
    }
}

pub(super) fn rules_to_validator_expr(
    rules: &[Rule],
    value_ty: &Type,
    element_ty: &Type,
) -> syn::Result<TokenStream2> {
    let exprs = rules
        .iter()
        .map(|rule| match rule {
            Rule::Min(expr) => Ok(quote!(::nebula_validator::validators::min(#expr))),
            Rule::Max(expr) => Ok(quote!(::nebula_validator::validators::max(#expr))),
            Rule::ExactLength(n) => Ok(quote!(::nebula_validator::validators::exact_length(#n))),
            Rule::MinLength(n) => Ok(quote!(::nebula_validator::validators::min_length(#n))),
            Rule::MaxLength(n) => Ok(quote!(::nebula_validator::validators::max_length(#n))),
            Rule::LengthRange { min, max } => Ok(quote!(::nebula_validator::validators::length_range(#min, #max).expect("length bounds validated by derive parser"))),
            Rule::ExactSize(n) => Ok(quote!(::nebula_validator::validators::exact_size::<#element_ty>(#n))),
            Rule::MinSize(n) => Ok(quote!(::nebula_validator::validators::min_size::<#element_ty>(#n))),
            Rule::MaxSize(n) => Ok(quote!(::nebula_validator::validators::max_size::<#element_ty>(#n))),
            Rule::SizeRange { min, max } => Ok(quote!(::nebula_validator::validators::try_size_range::<#element_ty>(#min, #max).expect("size bounds validated by derive parser"))),
            _ => Err(syn::Error::new_spanned(value_ty, "rule cannot be converted into a grouped validator expression")),
        })
        .collect::<syn::Result<Vec<_>>>()?;

    chain_validator_exprs(exprs, true)
}

pub(super) fn chain_validator_exprs(
    exprs: Vec<TokenStream2>,
    is_and: bool,
) -> syn::Result<TokenStream2> {
    let mut iter = exprs.into_iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "validator group requires at least one rule",
        ));
    };

    let combined = iter.fold(first, |left, right| {
        if is_and {
            quote!(::nebula_validator::combinators::and(#left, #right))
        } else {
            quote!(::nebula_validator::combinators::or(#left, #right))
        }
    });

    Ok(combined)
}

/// Parse a `key(min = N, max = M)` list attribute into a `(min, max)` pair.
pub(super) fn parse_min_max_list(
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
            },
            other => {
                return Err(diag::error_spanned(
                    &value_token(&other),
                    format!("`{key}` bounds must be integer literals"),
                ));
            },
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

/// Parse `each(...)` / `inner(...)` sub-attributes into an `AttrArgs`.
pub(super) fn parse_inner_args(
    validate_attrs: &attrs::AttrArgs,
) -> syn::Result<Option<attrs::AttrArgs>> {
    let mut items = Vec::new();
    for item in &validate_attrs.items {
        let attrs::AttrItem::List { key, values } = item else {
            continue;
        };
        if key != "each" && key != "inner" {
            continue;
        }

        for value in values {
            items.push(parse_list_item_to_attr_item(value)?);
        }
    }

    if items.is_empty() {
        return Ok(None);
    }

    Ok(Some(attrs::AttrArgs { items }))
}

/// Convert a list item value into an `AttrItem`.
pub(super) fn parse_list_item_to_attr_item(
    item: &attrs::AttrValue,
) -> syn::Result<attrs::AttrItem> {
    match item {
        attrs::AttrValue::Ident(ident) => Ok(attrs::AttrItem::Flag(ident.clone())),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid each(...) entry: {e}")))?;
            parse_expr_to_attr_item(parsed, item)
        },
        attrs::AttrValue::Tokens(tokens) => {
            let parsed = syn::parse2::<syn::Expr>(tokens.clone()).map_err(|e| {
                diag::error_spanned(tokens, format!("invalid each(...) entry: {e}"))
            })?;
            parse_expr_to_attr_item(parsed, item)
        },
        attrs::AttrValue::Lit(other) => Err(diag::error_spanned(
            other,
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

/// Convert a parsed expression into an `AttrItem`.
pub(super) fn parse_expr_to_attr_item(
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
        },
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
        },
        syn::Expr::Call(call) => {
            let syn::Expr::Path(func_path) = *call.func else {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) nested lists must use identifier keys",
                ));
            };

            if func_path.path.segments.len() != 1 || func_path.path.leading_colon.is_some() {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) nested lists must use identifier keys",
                ));
            }

            let key = func_path
                .path
                .segments
                .into_iter()
                .next()
                .expect("segment")
                .ident;
            let values = call.args.into_iter().map(expr_to_attr_value).collect();
            Ok(attrs::AttrItem::List { key, values })
        },
        _ => Err(diag::error_spanned(
            &value_token(span_source),
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

/// Convert an expression into an `AttrValue`.
pub(super) fn expr_to_attr_value(expr: syn::Expr) -> attrs::AttrValue {
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
        },
        syn::Expr::Lit(lit) => attrs::AttrValue::Lit(lit.lit),
        other => attrs::AttrValue::Tokens(quote!(#other)),
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn required_on_non_option_field_is_rejected() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(required)]
                email: String,
            }
        };

        let err = parse(&input).expect_err("required on String must fail");
        assert!(
            err.to_string()
                .contains("`required` requires `Option<T>` fields")
        );
    }

    #[test]
    fn required_on_option_field_is_accepted() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(required)]
                email: Option<String>,
            }
        };

        let ir = parse(&input).expect("required on Option must parse");
        assert_eq!(ir.fields.len(), 1);
        assert!(matches!(ir.fields[0].rules.as_slice(), [Rule::Required]));
    }

    #[test]
    fn exact_length_conflicts_with_min_or_max_length() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(min_length = 3, exact_length = 5)]
                name: String,
            }
        };

        let err = parse(&input).expect_err("exact_length conflict must fail");
        assert!(
            err.to_string()
                .contains("`exact_length` cannot be combined with `min_length` or `max_length`")
        );
    }

    #[test]
    fn exact_size_conflicts_with_size_range() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(exact_size = 2, size_range(min = 1, max = 5))]
                tags: Vec<String>,
            }
        };

        let err = parse(&input).expect_err("exact_size conflict must fail");
        assert!(
            err.to_string()
                .contains("`size_range(...)` cannot be combined with `exact_size`")
        );
    }

    #[test]
    fn boolean_flags_are_mutually_exclusive() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(is_true, is_false)]
                accepted: bool,
            }
        };

        let err = parse(&input).expect_err("is_true + is_false must fail");
        assert!(
            err.to_string()
                .contains("`is_true` cannot be combined with `is_false`")
        );
    }

    #[test]
    fn each_required_is_accepted_for_option_elements() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(each(required))]
                flags: Vec<Option<bool>>,
            }
        };

        let ir = parse(&input).expect("each(required) on Vec<Option<T>> must parse");
        let each = ir.fields[0]
            .each_rules
            .as_ref()
            .expect("each rules expected");
        assert!(matches!(each.rules.as_slice(), [Rule::Required]));
    }

    #[test]
    fn each_required_is_rejected_for_non_option_elements() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(each(required))]
                flags: Vec<bool>,
            }
        };

        let err = parse(&input).expect_err("each(required) on Vec<T> must fail");
        assert!(
            err.to_string()
                .contains("`each(required)` requires `Vec<Option<T>>`")
        );
    }

    #[test]
    fn each_boolean_flags_are_mutually_exclusive() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(each(is_true, is_false))]
                flags: Vec<bool>,
            }
        };

        let err = parse(&input).expect_err("each is_true + is_false must fail");
        assert!(
            err.to_string()
                .contains("`each(is_true)` cannot be combined with `each(is_false)`")
        );
    }

    #[test]
    fn using_rule_is_parsed_for_field() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(using = my_validator)]
                name: String,
            }
        };

        let ir = parse(&input).expect("using rule must parse");
        assert!(matches!(ir.fields[0].rules.as_slice(), [Rule::Using(_)]));
    }

    #[test]
    fn using_rule_is_parsed_for_each() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(each(using = my_validator))]
                names: Vec<String>,
            }
        };

        let ir = parse(&input).expect("each using rule must parse");
        let each = ir.fields[0]
            .each_rules
            .as_ref()
            .expect("each rules expected");
        assert!(matches!(each.rules.as_slice(), [Rule::Using(_)]));
    }

    #[test]
    fn all_rule_is_parsed_for_field() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(all(v1, v2))]
                name: String,
            }
        };

        let ir = parse(&input).expect("all rule must parse");
        assert!(matches!(ir.fields[0].rules.as_slice(), [Rule::All(exprs)] if exprs.len() == 2));
    }

    #[test]
    fn any_rule_is_parsed_for_each() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(each(any(v1, v2, v3)))]
                names: Vec<String>,
            }
        };

        let ir = parse(&input).expect("each any rule must parse");
        let each = ir.fields[0]
            .each_rules
            .as_ref()
            .expect("each rules expected");
        assert!(matches!(each.rules.as_slice(), [Rule::Any(exprs)] if exprs.len() == 3));
    }

    #[test]
    fn length_call_is_parsed_for_field() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(length(6))]
                code: String,
            }
        };

        let ir = parse(&input).expect("length call must parse");
        assert!(matches!(
            ir.fields[0].rules.as_slice(),
            [Rule::ExactLength(6)]
        ));
    }

    #[test]
    fn inner_alias_is_parsed_for_collection_elements() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(inner(length(2)))]
                tags: Vec<String>,
            }
        };

        let ir = parse(&input).expect("inner alias must parse");
        let each = ir.fields[0]
            .each_rules
            .as_ref()
            .expect("inner rules expected");
        assert!(matches!(each.rules.as_slice(), [Rule::ExactLength(2)]));
    }

    #[test]
    fn inner_nested_is_parsed_for_nested_elements() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(inner(nested()))]
                addresses: Vec<Address>,
            }
        };

        let ir = parse(&input).expect("inner nested must parse");
        let each = ir.fields[0]
            .each_rules
            .as_ref()
            .expect("inner rules expected");
        assert!(matches!(each.rules.as_slice(), [Rule::Nested]));
    }

    #[test]
    fn required_call_on_non_option_field_is_rejected() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(required())]
                email: String,
            }
        };

        let err = parse(&input).expect_err("required() on String must fail");
        assert!(
            err.to_string()
                .contains("`required()` requires `Option<T>` values")
        );
    }

    #[test]
    fn email_call_on_non_string_field_is_rejected() {
        let input: DeriveInput = parse_quote! {
            struct User {
                #[validate(email())]
                email: u32,
            }
        };

        let err = parse(&input).expect_err("email() on u32 must fail");
        assert!(
            err.to_string()
                .contains("`email()` requires `String` or `Option<String>` fields")
        );
    }
}
