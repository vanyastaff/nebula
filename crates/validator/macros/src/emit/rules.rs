//! Field-level rule emitters.
//!
//! Each rule variant has a dedicated emitter that produces the validation
//! code block for a single `#[validate(...)]` clause. The top-level
//! [`emit_field_rule`] dispatches to the appropriate emitter; shared
//! helpers like [`super::wrap_option`] and [`super::wrap_message`] handle
//! the option-unwrap and message-override plumbing.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::{
    string_factory_to_tokens, string_format_to_tokens, vec_inner_type_from_field, wrap_message,
    wrap_option,
};
use crate::model::{FieldDef, Rule};

// ---------------------------------------------------------------------------
// Per-rule codegen
// ---------------------------------------------------------------------------

/// Generate the check code for a single field-level rule.
pub(super) fn emit_field_rule(field: &FieldDef, rule: &Rule) -> TokenStream2 {
    match rule {
        Rule::Required => emit_required(field),
        Rule::MinLength(n) => emit_len_check(field, *n, true),
        Rule::MaxLength(n) => emit_len_check(field, *n, false),
        Rule::ExactLength(n) => emit_exact_len_check(field, *n),
        Rule::LengthRange { min, max } => emit_length_range(field, *min, *max),
        Rule::Min(bound) => emit_cmp_check(field, bound, true, false),
        Rule::Max(bound) => emit_cmp_check(field, bound, false, false),
        Rule::GreaterThan(bound) => emit_cmp_check(field, bound, true, true),
        Rule::LessThan(bound) => emit_cmp_check(field, bound, false, true),
        Rule::MinSize(n) => emit_size_validator(field, "min_size", *n),
        Rule::MaxSize(n) => emit_size_validator(field, "max_size", *n),
        Rule::ExactSize(n) => emit_size_validator(field, "exact_size", *n),
        Rule::SizeRange { min, max } => emit_size_range(field, *min, *max),
        Rule::NotEmptyCollection => emit_not_empty_collection(field),
        Rule::StringFormat(fmt) => emit_str_validator(field, string_format_to_tokens(*fmt)),
        Rule::StringFactory { kind, arg } => {
            emit_str_validator(field, string_factory_to_tokens(*kind, arg))
        },
        Rule::IsTrue => {
            emit_bool_validator(field, quote!(::nebula_validator::validators::is_true()))
        },
        Rule::IsFalse => {
            emit_bool_validator(field, quote!(::nebula_validator::validators::is_false()))
        },
        Rule::Regex(pattern) => emit_regex_validator(field, pattern),
        Rule::Nested => emit_nested_validator(field),
        Rule::Custom(expr) => emit_custom_validator(field, expr),
        Rule::Using(expr) => emit_using_validator(field, expr),
        Rule::All(exprs) => emit_all_validators(field, exprs),
        Rule::Any(exprs) => emit_any_validators(field, exprs),
    }
}

// ---------------------------------------------------------------------------
// Required — special case: no Option unwrapping
// ---------------------------------------------------------------------------

/// Emit the `required` check. This checks `input.field.is_none()` directly
/// without the Option-unwrapping pattern.
fn emit_required(field: &FieldDef) -> TokenStream2 {
    let field_name = &field.ident;
    let field_key = field_name.to_string();

    if let Some(message) = &field.message {
        quote! {
            if input.#field_name.is_none() {
                let mut err = ::nebula_validator::foundation::ValidationError::required(#field_key);
                err.message = ::std::borrow::Cow::Owned(#message.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if input.#field_name.is_none() {
                errors.add(::nebula_validator::foundation::ValidationError::required(#field_key));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Length checks (min_length, max_length, exact_length)
// ---------------------------------------------------------------------------

/// Emit min_length or max_length check.
///
/// Uses `wrap_option` — inner code references `value` uniformly.
fn emit_len_check(field: &FieldDef, bound: usize, is_min: bool) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let error = if is_min {
        quote! {
            ::nebula_validator::foundation::ValidationError::min_length(
                #field_key, #bound, value.len(),
            )
        }
    } else {
        quote! {
            ::nebula_validator::foundation::ValidationError::max_length(
                #field_key, #bound, value.len(),
            )
        }
    };

    let cmp = if is_min {
        quote!(value.len() < #bound)
    } else {
        quote!(value.len() > #bound)
    };

    let inner = quote! {
        if #cmp {
            errors.add(#error);
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

/// Emit exact_length check.
fn emit_exact_len_check(field: &FieldDef, expected: usize) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        if value.len() != #expected {
            errors.add(::nebula_validator::foundation::ValidationError::exact_length(
                #field_key, #expected, value.len(),
            ));
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Length range
// ---------------------------------------------------------------------------

/// Emit `length_range(min, max)` check.
fn emit_length_range(field: &FieldDef, min: usize, max: usize) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        match ::nebula_validator::validators::length_range(#min, #max) {
            Ok(v) => {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                    errors.add(e.with_field(#field_key));
                }
            }
            Err(e) => {
                errors.add(e.with_field(#field_key));
            }
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Numeric comparison (min, max)
// ---------------------------------------------------------------------------

/// Emit a numeric comparison check for `min` / `max` / `greater_than` /
/// `less_than` rules. `is_min` picks the direction, `is_exclusive` picks
/// strict (`>` / `<`) vs inclusive (`>=` / `<=`) semantics.
fn emit_cmp_check(
    field: &FieldDef,
    bound: &TokenStream2,
    is_min: bool,
    is_exclusive: bool,
) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let (code, cmp, op) = match (is_min, is_exclusive) {
        (true, false) => ("min", quote!(value < &#bound), ">="),
        (false, false) => ("max", quote!(value > &#bound), "<="),
        (true, true) => ("greater_than", quote!(value <= &#bound), ">"),
        (false, true) => ("less_than", quote!(value >= &#bound), "<"),
    };

    let inner = quote! {
        if #cmp {
            errors.add(
                ::nebula_validator::foundation::ValidationError::new(
                    #code, format!("{} must be {} {}", #field_key, #op, #bound),
                ).with_field(#field_key)
            );
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Collection size validators (min_size, max_size, exact_size)
// ---------------------------------------------------------------------------

/// Emit a collection size validator check (min_size, max_size, exact_size).
fn emit_size_validator(field: &FieldDef, validator_name: &str, size: usize) -> TokenStream2 {
    let field_key = field.ident.to_string();
    let element_type = vec_inner_type_from_field(field);
    let validator_ident = syn::Ident::new(validator_name, proc_macro2::Span::call_site());

    let inner = quote! {
        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
            &::nebula_validator::validators::#validator_ident::<#element_type>(#size),
            value.as_slice(),
        ) {
            errors.add(e.with_field(#field_key));
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

/// Emit `size_range(min, max)` check.
fn emit_size_range(field: &FieldDef, min: usize, max: usize) -> TokenStream2 {
    let field_key = field.ident.to_string();
    let element_type = vec_inner_type_from_field(field);

    let inner = quote! {
        match ::nebula_validator::validators::try_size_range::<#element_type>(#min, #max) {
            Ok(v) => {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                    &v,
                    value.as_slice(),
                ) {
                    errors.add(e.with_field(#field_key));
                }
            }
            Err(e) => {
                errors.add(e.with_field(#field_key));
            }
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

/// Emit `not_empty_collection` check.
fn emit_not_empty_collection(field: &FieldDef) -> TokenStream2 {
    let field_key = field.ident.to_string();
    let element_type = vec_inner_type_from_field(field);

    let inner = quote! {
        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
            &::nebula_validator::validators::not_empty_collection::<#element_type>(),
            value.as_slice(),
        ) {
            errors.add(e.with_field(#field_key));
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// String validators
// ---------------------------------------------------------------------------

/// Emit a string validator check using a built-in validator expression.
fn emit_str_validator(field: &FieldDef, validator_expr: TokenStream2) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
            errors.add(e.with_field(#field_key));
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Bool validators
// ---------------------------------------------------------------------------

/// Emit a boolean validator check.
fn emit_bool_validator(field: &FieldDef, validator_expr: TokenStream2) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value) {
            errors.add(e.with_field(#field_key));
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Regex validator
// ---------------------------------------------------------------------------

/// Emit a regex validator check.
///
/// The pattern is pre-compiled once per process via `LazyLock<Regex>`; the
/// block introduced by [`wrap_option`] gives each field its own scope so
/// multiple regex fields in the same struct do not collide on the `RE`
/// identifier. Pattern validity is verified at macro-time (see
/// `parse::*::parse_regex_pattern`), so the `expect` here is defensive.
fn emit_regex_validator(field: &FieldDef, pattern: &str) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        static RE: ::std::sync::LazyLock<::nebula_validator::__private::regex::Regex> =
            ::std::sync::LazyLock::new(|| {
                ::nebula_validator::__private::regex::Regex::new(#pattern)
                    .expect("nebula-validator: regex validated at macro time")
            });
        if !RE.is_match(value.as_str()) {
            errors.add(
                ::nebula_validator::foundation::ValidationError::invalid_format("", "regex")
                    .with_param("pattern", #pattern.to_string())
                    .with_field(#field_key),
            );
        }
    };

    wrap_message(field, wrap_option(field, inner))
}

// ---------------------------------------------------------------------------
// Nested validator
// ---------------------------------------------------------------------------

/// Emit a nested validation check via `SelfValidating::check`.
fn emit_nested_validator(field: &FieldDef) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = if let Some(message) = &field.message {
        quote! {
            if let Err(mut e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                e = e.with_field(#field_key);
                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                errors.add(e.with_field(#field_key));
            }
        }
    };

    // NOTE: nested uses its own message pattern (mut e) instead of wrap_message,
    // because it needs to set both field and message on the error before adding.
    // wrap_option is still used to centralize Option handling.
    wrap_option(field, inner)
}

// ---------------------------------------------------------------------------
// Custom validator
// ---------------------------------------------------------------------------

/// Emit a custom validator expression check.
fn emit_custom_validator(field: &FieldDef, expr: &TokenStream2) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = if let Some(message) = &field.message {
        quote! {
            if let Err(mut e) = (#expr)(value) {
                e = e.with_field(#field_key);
                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = (#expr)(value) {
                errors.add(e.with_field(#field_key));
            }
        }
    };

    // NOTE: custom uses its own message pattern (mut e) instead of wrap_message,
    // same as nested — needs to modify the error before adding.
    wrap_option(field, inner)
}

/// Emit a validator-expression check via `Validate::validate`.
fn emit_using_validator(field: &FieldDef, expr: &TokenStream2) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = if let Some(message) = &field.message {
        quote! {
            if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                e = e.with_field(#field_key);
                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                errors.add(e.with_field(#field_key));
            }
        }
    };

    wrap_option(field, inner)
}

/// Emit `all(v1, v2, ...)` by applying validators sequentially.
fn emit_all_validators(field: &FieldDef, exprs: &[TokenStream2]) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let checks: Vec<TokenStream2> = exprs
        .iter()
        .map(|expr| {
            quote! {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                    errors.add(e.with_field(#field_key));
                }
            }
        })
        .collect();

    let inner = quote! {
        #(#checks)*
    };

    wrap_message(field, wrap_option(field, inner))
}

/// Emit `any(v1, v2, ...)` by accepting the first passing validator.
fn emit_any_validators(field: &FieldDef, exprs: &[TokenStream2]) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let attempts: Vec<TokenStream2> = exprs
        .iter()
        .map(|expr| {
            quote! {
                if !__nebula_any_passed {
                    match ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                        Ok(()) => {
                            __nebula_any_passed = true;
                        }
                        Err(e) => {
                            __nebula_any_errors.add(e.with_field(#field_key));
                        }
                    }
                }
            }
        })
        .collect();

    let inner = quote! {
        let mut __nebula_any_passed = false;
        let mut __nebula_any_errors = ::nebula_validator::foundation::ValidationErrors::new();
        #(#attempts)*

        if !__nebula_any_passed {
            let count = __nebula_any_errors.len();
            errors.add(
                ::nebula_validator::foundation::ValidationError::new(
                    "any_failed",
                    format!("all {} validators in any(...) failed", count),
                )
                .with_field(#field_key)
                .with_nested(__nebula_any_errors.into_iter().collect()),
            );
        }
    };

    wrap_message(field, wrap_option(field, inner))
}
