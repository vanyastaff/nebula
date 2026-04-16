//! Field-level `#[validate(...)]` rule parsing.
//!
//! Converts a field's attribute args into a list of [`Rule`] IR nodes,
//! delegating to shared helpers in [`super`] for type checks, validator-
//! expression parsing, and call-style argument handling.

use nebula_macro_support::{
    attrs,
    validation_codegen::{is_option_type, parse_number_lit, parse_usize},
};
use syn::Type;

use super::{
    parse_call_style_rules, parse_min_max_list, parse_validator_expr, parse_validator_expr_list,
    require_bool_type, require_string_type, require_vec_type, string_factory_keys,
    string_format_flags,
};
use crate::{
    model::Rule,
    types::{is_bool_type, is_string_type, vec_inner_type},
};

/// Parse all `#[validate(...)]` attributes on a field into a list of [`Rule`]s.
pub(super) fn parse_field_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Vec<Rule>> {
    let is_option = is_option_type(original_ty);
    let is_string = is_string_type(inner_ty);
    let is_bool = is_bool_type(inner_ty);
    let is_vec = vec_inner_type(inner_ty).is_some();

    let mut rules = Vec::new();

    // required — only valid for Option fields
    if attrs.has_flag("required") {
        if is_option {
            rules.push(Rule::Required);
        } else {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`required` requires `Option<T>` fields",
            ));
        }
    }

    // min_length / max_length / exact_length
    let min_length = parse_usize(attrs, "min_length")?;
    let max_length = parse_usize(attrs, "max_length")?;
    let exact_length = parse_usize(attrs, "exact_length")?;

    if exact_length.is_some() && (min_length.is_some() || max_length.is_some()) {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`exact_length` cannot be combined with `min_length` or `max_length`",
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

    // length_range(min = N, max = M)
    let length_range = parse_min_max_list(attrs, "length_range")?;
    if length_range.is_some() && exact_length.is_some() {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`length_range(...)` cannot be combined with `exact_length`",
        ));
    }

    if let Some((min, max)) = length_range {
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
    let min_size = parse_usize(attrs, "min_size")?;
    let max_size = parse_usize(attrs, "max_size")?;
    let exact_size = parse_usize(attrs, "exact_size")?;

    if exact_size.is_some() && (min_size.is_some() || max_size.is_some()) {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`exact_size` cannot be combined with `min_size` or `max_size`",
        ));
    }

    if let Some(n) = min_size {
        require_vec_type(original_ty, is_vec, "min_size")?;
        rules.push(Rule::MinSize(n));
    }
    if let Some(n) = max_size {
        require_vec_type(original_ty, is_vec, "max_size")?;
        rules.push(Rule::MaxSize(n));
    }
    if let Some(n) = exact_size {
        require_vec_type(original_ty, is_vec, "exact_size")?;
        rules.push(Rule::ExactSize(n));
    }

    // not_empty_collection
    if attrs.has_flag("not_empty_collection") {
        require_vec_type(original_ty, is_vec, "not_empty_collection")?;
        rules.push(Rule::NotEmptyCollection);
    }

    // size_range(min = N, max = M)
    let size_range = parse_min_max_list(attrs, "size_range")?;
    if size_range.is_some() && exact_size.is_some() {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`size_range(...)` cannot be combined with `exact_size`",
        ));
    }

    if let Some((min, max)) = size_range {
        require_vec_type(original_ty, is_vec, "size_range(...)")?;
        rules.push(Rule::SizeRange { min, max });
    }

    // String format flags (not_empty, email, url, etc.)
    for (flag, format) in string_format_flags().iter().copied() {
        if attrs.has_flag(flag) {
            require_string_type(original_ty, is_string, flag)?;
            rules.push(Rule::StringFormat(format));
        }
    }

    // String factory keys (contains, starts_with, ends_with)
    for (key, kind) in string_factory_keys().iter().copied() {
        if let Some(arg) = attrs.get_string(key) {
            require_string_type(original_ty, is_string, &format!("{key} = ..."))?;
            rules.push(Rule::StringFactory { kind, arg });
        }
    }

    // is_true / is_false
    let has_is_true = attrs.has_flag("is_true");
    let has_is_false = attrs.has_flag("is_false");
    if has_is_true && has_is_false {
        return Err(syn::Error::new_spanned(
            original_ty,
            "`is_true` cannot be combined with `is_false`",
        ));
    }

    if has_is_true {
        require_bool_type(original_ty, is_bool, "is_true")?;
        rules.push(Rule::IsTrue);
    }
    if has_is_false {
        require_bool_type(original_ty, is_bool, "is_false")?;
        rules.push(Rule::IsFalse);
    }

    // regex = "pattern"
    if let Some(pattern) = attrs.get_string("regex") {
        require_string_type(original_ty, is_string, "regex = ...")?;
        super::validate_regex_pattern(&pattern, original_ty)?;
        rules.push(Rule::Regex(pattern));
    }

    // nested
    if attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom = expr
    if let Some(expr) = parse_validator_expr(attrs, "custom")? {
        rules.push(Rule::Custom(expr));
    }

    // using = expr
    if let Some(expr) = parse_validator_expr(attrs, "using")? {
        rules.push(Rule::Using(expr));
    }

    // all(expr, ...)
    if let Some(exprs) = parse_validator_expr_list(attrs, "all")? {
        rules.push(Rule::All(exprs));
    }

    // any(expr, ...)
    if let Some(exprs) = parse_validator_expr_list(attrs, "any")? {
        rules.push(Rule::Any(exprs));
    }

    rules.extend(parse_call_style_rules(
        attrs,
        original_ty,
        inner_ty,
        is_option,
        is_string,
        is_bool,
        is_vec,
    )?);

    Ok(rules)
}
