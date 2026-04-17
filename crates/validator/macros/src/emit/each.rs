//! `each(...)` loop emitters — per-element validation for `Vec<T>` fields.
//!
//! [`emit_each_loop`] generates the outer `for` loop; [`emit_each_rule`]
//! dispatches per-element rules to their dedicated emitters.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::{string_factory_to_tokens, string_format_to_tokens};
use crate::{
    model::{EachRules, FieldDef, Rule},
    types::is_option_type as option_type_check,
};

// ---------------------------------------------------------------------------
// Each-loop codegen
// ---------------------------------------------------------------------------

/// Generate the `for (index, value) in collection.iter().enumerate()` loop
/// for `each(...)` rules.
pub(super) fn emit_each_loop(field: &FieldDef, each: &EachRules) -> TokenStream2 {
    let field_name = &field.ident;
    let field_key = field_name.to_string();

    let each_checks: Vec<TokenStream2> = each
        .rules
        .iter()
        .map(|rule| emit_each_rule(rule, field, each))
        .collect();

    let each_loop = quote! {
        for (index, value) in collection.iter().enumerate() {
            let each_field = format!("{}[{}]", #field_key, index);
            #(#each_checks)*
        }
    };

    if option_type_check(&field.ty) {
        quote! {
            if let Some(collection) = input.#field_name.as_ref() {
                #each_loop
            }
        }
    } else {
        quote! {
            let collection = &input.#field_name;
            #each_loop
        }
    }
}

/// Generate the check code for a single element-level rule inside the each loop.
///
/// Inside the loop, `value` is the element ref and `each_field` is the indexed
/// field path string (e.g. `"tags[0]"`).
fn emit_each_rule(rule: &Rule, field: &FieldDef, each: &EachRules) -> TokenStream2 {
    let message = &field.message;
    let element_is_option = option_type_check(&each.element_ty);

    match rule {
        Rule::Required => emit_each_required(message, element_is_option),
        Rule::MinLength(n) => emit_each_len_check(*n, true, message, element_is_option),
        Rule::MaxLength(n) => emit_each_len_check(*n, false, message, element_is_option),
        Rule::ExactLength(n) => emit_each_exact_len(*n, message, element_is_option),
        Rule::Min(bound) => emit_each_cmp_check(bound, true, false, message, element_is_option),
        Rule::Max(bound) => emit_each_cmp_check(bound, false, false, message, element_is_option),
        Rule::GreaterThan(bound) => {
            emit_each_cmp_check(bound, true, true, message, element_is_option)
        },
        Rule::LessThan(bound) => {
            emit_each_cmp_check(bound, false, true, message, element_is_option)
        },
        Rule::StringFormat(fmt) => {
            emit_each_str_validator(string_format_to_tokens(*fmt), message, element_is_option)
        },
        Rule::StringFactory { kind, arg } => emit_each_str_validator(
            string_factory_to_tokens(*kind, arg),
            message,
            element_is_option,
        ),
        Rule::IsTrue => emit_each_bool_check(true, message, element_is_option),
        Rule::IsFalse => emit_each_bool_check(false, message, element_is_option),
        Rule::Regex(pattern) => emit_each_regex(pattern, message, element_is_option),
        Rule::Nested => emit_each_nested(message, element_is_option),
        Rule::Custom(expr) => emit_each_custom(expr, message, element_is_option),
        Rule::Using(expr) => emit_each_using(expr, message, element_is_option),
        Rule::All(exprs) => emit_each_all(exprs, message, element_is_option),
        Rule::Any(exprs) => emit_each_any(exprs, message, element_is_option),
        // Other rules are not valid inside each(...) but parse phase would
        // not produce them; this arm is for completeness.
        _ => quote!(),
    }
}

// ---------------------------------------------------------------------------
// Each-loop per-rule helpers
// ---------------------------------------------------------------------------

/// Emit element-level min_length or max_length check.
fn emit_each_len_check(
    bound: usize,
    is_min: bool,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let error_ctor = if is_min {
        quote!(::nebula_validator::foundation::ValidationError::min_length)
    } else {
        quote!(::nebula_validator::foundation::ValidationError::max_length)
    };
    let cmp = if is_min {
        quote!(value.len() < #bound)
    } else {
        quote!(value.len() > #bound)
    };

    let check = if let Some(msg) = message {
        quote! {
            if #cmp {
                let mut err = #error_ctor(each_field.clone(), #bound, value.len());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if #cmp {
                errors.add(#error_ctor(each_field.clone(), #bound, value.len()));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level exact_length check.
fn emit_each_exact_len(
    expected: usize,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            if value.len() != #expected {
                let mut err = ::nebula_validator::foundation::ValidationError::exact_length(
                    each_field.clone(), #expected, value.len(),
                );
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if value.len() != #expected {
                errors.add(::nebula_validator::foundation::ValidationError::exact_length(
                    each_field.clone(), #expected, value.len(),
                ));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level numeric comparison check. `is_min` chooses direction,
/// `is_exclusive` picks strict vs inclusive comparison.
fn emit_each_cmp_check(
    bound: &TokenStream2,
    is_min: bool,
    is_exclusive: bool,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let (code, cmp, op_str) = match (is_min, is_exclusive) {
        (true, false) => ("min", quote!(value < &#bound), ">="),
        (false, false) => ("max", quote!(value > &#bound), "<="),
        (true, true) => ("greater_than", quote!(value <= &#bound), ">"),
        (false, true) => ("less_than", quote!(value >= &#bound), "<"),
    };

    let check = if let Some(msg) = message {
        quote! {
            if #cmp {
                let mut err = ::nebula_validator::foundation::ValidationError::new(
                    #code,
                    format!("{} must be {} {}", each_field, #op_str, #bound),
                )
                .with_field(each_field.clone());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if #cmp {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(
                        #code,
                        format!("{} must be {} {}", each_field, #op_str, #bound),
                    )
                    .with_field(each_field.clone()),
                );
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level string validator check.
fn emit_each_str_validator(
    validator_expr: TokenStream2,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                e = e.with_field(each_field.clone());
                e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                errors.add(e.with_field(each_field.clone()));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level regex validator check.
///
/// Mirrors the field-level emitter (`rules::emit_regex_validator`): the
/// pattern is pre-compiled once via `LazyLock<Regex>`; being inside the
/// `for (index, _)` loop body gives each element check its own scope.
fn emit_each_regex(
    pattern: &str,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            static RE: ::std::sync::LazyLock<::nebula_validator::__private::regex::Regex> =
                ::std::sync::LazyLock::new(|| {
                    ::nebula_validator::__private::regex::Regex::new(#pattern)
                        .expect("nebula-validator: regex validated at macro time")
                });
            if !RE.is_match(value.as_str()) {
                let mut err = ::nebula_validator::foundation::ValidationError::invalid_format(
                    "",
                    "regex",
                )
                .with_param("pattern", #pattern.to_string())
                .with_field(each_field.clone());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            static RE: ::std::sync::LazyLock<::nebula_validator::__private::regex::Regex> =
                ::std::sync::LazyLock::new(|| {
                    ::nebula_validator::__private::regex::Regex::new(#pattern)
                        .expect("nebula-validator: regex validated at macro time")
                });
            if !RE.is_match(value.as_str()) {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::invalid_format("", "regex")
                        .with_param("pattern", #pattern.to_string())
                        .with_field(each_field.clone()),
                );
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level nested validation check.
fn emit_each_nested(message: &Option<String>, element_is_option: bool) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            if let Err(mut e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                e = e.with_field(each_field.clone());
                e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                errors.add(e.with_field(each_field.clone()));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level custom validator check.
fn emit_each_custom(
    expr: &TokenStream2,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            if let Err(mut e) = (#expr)(value) {
                e = e.with_field(each_field.clone());
                e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = (#expr)(value) {
                errors.add(e.with_field(each_field.clone()));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit element-level validator-expression check.
fn emit_each_using(
    expr: &TokenStream2,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let check = if let Some(msg) = message {
        quote! {
            if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                e = e.with_field(each_field.clone());
                e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(e);
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                errors.add(e.with_field(each_field.clone()));
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit `each(all(...))` by applying all validators to every element.
fn emit_each_all(
    exprs: &[TokenStream2],
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let checks: Vec<TokenStream2> = exprs
        .iter()
        .map(|expr| {
            if let Some(msg) = message {
                quote! {
                    if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                        e = e.with_field(each_field.clone());
                        e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                        errors.add(e);
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                        errors.add(e.with_field(each_field.clone()));
                    }
                }
            }
        })
        .collect();

    let check = quote! {
        #(#checks)*
    };

    wrap_each_option(element_is_option, check)
}

/// Emit `each(any(...))` by accepting the first passing validator per element.
fn emit_each_any(
    exprs: &[TokenStream2],
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let attempts: Vec<TokenStream2> = exprs
        .iter()
        .map(|expr| {
            quote! {
                if !__nebula_each_any_passed {
                    match ::nebula_validator::foundation::Validate::validate(&(#expr), value) {
                        Ok(()) => {
                            __nebula_each_any_passed = true;
                        }
                        Err(e) => {
                            __nebula_each_any_errors.add(e.with_field(each_field.clone()));
                        }
                    }
                }
            }
        })
        .collect();

    let check = if let Some(msg) = message {
        quote! {
            let mut __nebula_each_any_passed = false;
            let mut __nebula_each_any_errors = ::nebula_validator::foundation::ValidationErrors::new();
            #(#attempts)*

            if !__nebula_each_any_passed {
                let count = __nebula_each_any_errors.len();
                let mut err = ::nebula_validator::foundation::ValidationError::new(
                    "any_failed",
                    format!("all {} validators in any(...) failed", count),
                )
                .with_field(each_field.clone())
                .with_nested(__nebula_each_any_errors.into_iter().collect());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            let mut __nebula_each_any_passed = false;
            let mut __nebula_each_any_errors = ::nebula_validator::foundation::ValidationErrors::new();
            #(#attempts)*

            if !__nebula_each_any_passed {
                let count = __nebula_each_any_errors.len();
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(
                        "any_failed",
                        format!("all {} validators in any(...) failed", count),
                    )
                    .with_field(each_field.clone())
                    .with_nested(__nebula_each_any_errors.into_iter().collect()),
                );
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Emit `required` for each element (`Vec<Option<T>>` only).
fn emit_each_required(message: &Option<String>, element_is_option: bool) -> TokenStream2 {
    if !element_is_option {
        return quote!();
    }

    if let Some(msg) = message {
        quote! {
            if value.is_none() {
                let mut err = ::nebula_validator::foundation::ValidationError::required(each_field.clone());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if value.is_none() {
                errors.add(::nebula_validator::foundation::ValidationError::required(each_field.clone()));
            }
        }
    }
}

/// Emit bool checks for each element.
fn emit_each_bool_check(
    expect_true: bool,
    message: &Option<String>,
    element_is_option: bool,
) -> TokenStream2 {
    let (code, default_msg, condition) = if expect_true {
        ("is_true", "Value must be true", quote!(!*value))
    } else {
        ("is_false", "Value must be false", quote!(*value))
    };

    let check = if let Some(msg) = message {
        quote! {
            if #condition {
                let mut err = ::nebula_validator::foundation::ValidationError::new(#code, #default_msg)
                    .with_field(each_field.clone());
                err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                errors.add(err);
            }
        }
    } else {
        quote! {
            if #condition {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(#code, #default_msg)
                        .with_field(each_field.clone()),
                );
            }
        }
    };

    wrap_each_option(element_is_option, check)
}

/// Wrap each-rule checks so `Option` elements are validated only when `Some(...)`.
fn wrap_each_option(element_is_option: bool, check: TokenStream2) -> TokenStream2 {
    if element_is_option {
        quote! {
            if let Some(value) = value.as_ref() {
                #check
            }
        }
    } else {
        check
    }
}
