//! `each(...)` / `inner(...)` rule parsing for `Vec<T>` fields.
//!
//! Produces an [`EachRules`] value describing the per-element rules applied
//! to collection fields. Shares the same expression-parsing primitives with
//! field-level parsing in [`super::rules`].

use nebula_macro_support::{
    attrs,
    validation_codegen::{is_option_type, parse_number_lit, parse_usize},
};
use syn::Type;

use super::{
    parse_call_style_rules, parse_inner_args, parse_validator_expr, parse_validator_expr_list,
    string_factory_keys, string_format_flags,
};
use crate::{
    model::{EachRules, Rule},
    types::{is_bool_type, is_string_type, option_inner_type, vec_inner_type},
};

/// Parse `each(...)` sub-attributes into [`EachRules`].
pub(super) fn parse_each_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Option<EachRules>> {
    let Some(each_attrs) = parse_inner_args(attrs)? else {
        return Ok(None);
    };

    let element_source_type = option_inner_type(original_ty).unwrap_or(inner_ty);
    let element_ty = vec_inner_type(element_source_type).ok_or_else(|| {
        syn::Error::new_spanned(
            original_ty,
            "`inner(...)`/`each(...)` is supported for `Vec<T>` and `Option<Vec<T>>` fields",
        )
    })?;

    let each_element_is_option = is_option_type(element_ty);
    let each_inner_ty = option_inner_type(element_ty).unwrap_or(element_ty);
    let each_element_is_string = is_string_type(each_inner_ty);
    let each_element_is_bool = is_bool_type(each_inner_ty);
    let mut rules = Vec::new();

    // required on each element is meaningful only for Option elements
    if each_attrs.has_flag("required") {
        if each_element_is_option {
            rules.push(Rule::Required);
        } else {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`each(required)` requires `Vec<Option<T>>` or `Option<Vec<Option<T>>>`",
            ));
        }
    }

    // min_length / max_length / exact_length
    let min_length = parse_usize(&each_attrs, "min_length")?;
    let max_length = parse_usize(&each_attrs, "max_length")?;
    let exact_length = parse_usize(&each_attrs, "exact_length")?;

    if exact_length.is_some() && (min_length.is_some() || max_length.is_some()) {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`each(exact_length = ...)` cannot be combined with `each(min_length = ...)` or `each(max_length = ...)`",
        ));
    }

    if let Some(n) = min_length {
        rules.push(Rule::MinLength(n));
    }
    if let Some(n) = max_length {
        rules.push(Rule::MaxLength(n));
    }
    if let Some(n) = exact_length {
        rules.push(Rule::ExactLength(n));
    }

    // min / max (numeric, inclusive)
    if let Some(ts) = parse_number_lit(&each_attrs, "min")? {
        rules.push(Rule::Min(ts));
    }
    if let Some(ts) = parse_number_lit(&each_attrs, "max")? {
        rules.push(Rule::Max(ts));
    }

    // greater_than / less_than (numeric, exclusive)
    if let Some(ts) = parse_number_lit(&each_attrs, "greater_than")? {
        rules.push(Rule::GreaterThan(ts));
    }
    if let Some(ts) = parse_number_lit(&each_attrs, "less_than")? {
        rules.push(Rule::LessThan(ts));
    }

    let has_is_true = each_attrs.has_flag("is_true");
    let has_is_false = each_attrs.has_flag("is_false");
    if has_is_true && has_is_false {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`each(is_true)` cannot be combined with `each(is_false)`",
        ));
    }
    if has_is_true {
        if !each_element_is_bool {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`each(is_true)` requires `Vec<bool>`, `Vec<Option<bool>>`, or optional wrappers",
            ));
        }
        rules.push(Rule::IsTrue);
    }
    if has_is_false {
        if !each_element_is_bool {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`each(is_false)` requires `Vec<bool>`, `Vec<Option<bool>>`, or optional wrappers",
            ));
        }
        rules.push(Rule::IsFalse);
    }

    // String format flags
    for (flag, format) in string_format_flags().iter().copied() {
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
    for (key, kind) in string_factory_keys().iter().copied() {
        if let Some(arg) = each_attrs.get_string(key) {
            if !each_element_is_string {
                return Err(syn::Error::new_spanned(
                    original_ty,
                    format!("`each({key} = ...)` requires `Vec<String>` or `Option<Vec<String>>`"),
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
        super::validate_regex_pattern(&pattern, original_ty)?;
        rules.push(Rule::Regex(pattern));
    }

    // nested
    if each_attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom
    if let Some(expr) = parse_validator_expr(&each_attrs, "custom")? {
        rules.push(Rule::Custom(expr));
    }

    // using
    if let Some(expr) = parse_validator_expr(&each_attrs, "using")? {
        rules.push(Rule::Using(expr));
    }

    // all(...)
    if let Some(exprs) = parse_validator_expr_list(&each_attrs, "all")? {
        rules.push(Rule::All(exprs));
    }

    // any(...)
    if let Some(exprs) = parse_validator_expr_list(&each_attrs, "any")? {
        rules.push(Rule::Any(exprs));
    }

    rules.extend(parse_call_style_rules(
        &each_attrs,
        original_ty,
        each_inner_ty,
        each_element_is_option,
        each_element_is_string,
        each_element_is_bool,
        vec_inner_type(each_inner_ty).is_some(),
    )?);

    Ok(Some(EachRules {
        element_ty: element_ty.clone(),
        rules,
    }))
}
