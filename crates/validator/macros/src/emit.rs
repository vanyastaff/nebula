//! Emit phase: generates `TokenStream` from the [`ValidatorInput`] IR.
//!
//! The core architectural win: Option-wrapping and message-override are each
//! handled in ONE function instead of being duplicated across every rule.

#![allow(dead_code)] // Unused until Task 4 wires the 3-phase pipeline.
#![forbid(unsafe_code)]

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Type;

use crate::model::{
    EachRules, FieldDef, Rule, StringFactoryKind, StringFormat, ValidatorInput,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate the full `Validate`, `SelfValidating`, and `validate_fields()`
/// implementations from the parsed IR.
pub fn emit(input: &ValidatorInput) -> TokenStream2 {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let root_message = &input.container.message;

    let mut checks = Vec::new();
    for field in &input.fields {
        // Each-loop checks are emitted BEFORE field-level checks,
        // matching the old validator.rs ordering.
        if let Some(each) = &field.each_rules {
            checks.push(emit_each_loop(field, each));
        }
        for rule in &field.rules {
            checks.push(emit_field_rule(field, rule));
        }
    }

    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Validates this value using field-level `#[validate(...)]` rules.
            pub fn validate_fields(
                &self,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationErrors> {
                let input = self;
                let mut errors = ::nebula_validator::foundation::ValidationErrors::new();
                #(#checks)*

                if errors.has_errors() {
                    Err(errors)
                } else {
                    Ok(())
                }
            }
        }

        impl #impl_generics ::nebula_validator::foundation::Validate<#struct_name #ty_generics> for #struct_name #ty_generics #where_clause {
            fn validate(
                &self,
                input: &#struct_name #ty_generics,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                let _ = self;
                input
                    .validate_fields()
                    .map_err(|errors| errors.into_single_error(#root_message))
            }
        }

        impl #impl_generics ::nebula_validator::combinators::SelfValidating for #struct_name #ty_generics #where_clause {
            fn check(&self) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                self.validate(self)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Central wrappers — the architectural win
// ---------------------------------------------------------------------------

/// Bind `value` to a reference of the field's inner type.
///
/// - **Option fields:** `if let Some(value) = input.field.as_ref() { <check> }`
/// - **Non-option fields:** `let value = &input.field; <check>`
///
/// All inner check code uses `value` uniformly — no per-rule duplication.
fn wrap_option(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
    let field_name = &field.ident;
    if field.is_option {
        quote! {
            if let Some(value) = input.#field_name.as_ref() {
                #check
            }
        }
    } else {
        quote! {
            {
                let value = &input.#field_name;
                #check
            }
        }
    }
}

/// Wrap a check with the before/after/last_mut message override pattern
/// when the field has a `message = "..."` override.
fn wrap_message(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
    if let Some(message) = &field.message {
        quote! {
            let before = errors.len();
            #check
            let after = errors.len();
            if after > before {
                if let Some(last) = errors.last_mut() {
                    last.message = ::std::borrow::Cow::Owned(#message.to_string());
                }
            }
        }
    } else {
        check
    }
}

// ---------------------------------------------------------------------------
// Per-rule codegen
// ---------------------------------------------------------------------------

/// Generate the check code for a single field-level rule.
fn emit_field_rule(field: &FieldDef, rule: &Rule) -> TokenStream2 {
    match rule {
        Rule::Required => emit_required(field),
        Rule::MinLength(n) => emit_len_check(field, *n, true),
        Rule::MaxLength(n) => emit_len_check(field, *n, false),
        Rule::ExactLength(n) => emit_exact_len_check(field, *n),
        Rule::LengthRange { min, max } => emit_length_range(field, *min, *max),
        Rule::Min(bound) => emit_cmp_check(field, bound, true),
        Rule::Max(bound) => emit_cmp_check(field, bound, false),
        Rule::MinSize(n) => emit_size_validator(field, "min_size", *n),
        Rule::MaxSize(n) => emit_size_validator(field, "max_size", *n),
        Rule::ExactSize(n) => emit_size_validator(field, "exact_size", *n),
        Rule::SizeRange { min, max } => emit_size_range(field, *min, *max),
        Rule::NotEmptyCollection => emit_not_empty_collection(field),
        Rule::StringFormat(fmt) => emit_str_validator(field, string_format_to_tokens(*fmt)),
        Rule::StringFactory { kind, arg } => {
            emit_str_validator(field, string_factory_to_tokens(*kind, arg))
        }
        Rule::IsTrue => {
            emit_bool_validator(field, quote!(::nebula_validator::validators::is_true()))
        }
        Rule::IsFalse => {
            emit_bool_validator(field, quote!(::nebula_validator::validators::is_false()))
        }
        Rule::Regex(pattern) => emit_regex_validator(field, pattern),
        Rule::Nested => emit_nested_validator(field),
        Rule::Custom(expr) => emit_custom_validator(field, expr),
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

/// Emit min or max numeric comparison check.
fn emit_cmp_check(field: &FieldDef, bound: &TokenStream2, is_min: bool) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let (code, cmp) = if is_min {
        ("min", quote!(value < &#bound))
    } else {
        ("max", quote!(value > &#bound))
    };

    let op = if is_min { ">=" } else { "<=" };

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
        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
            &::nebula_validator::validators::size_range::<#element_type>(#min, #max),
            value.as_slice(),
        ) {
            errors.add(e.with_field(#field_key));
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
fn emit_regex_validator(field: &FieldDef, pattern: &str) -> TokenStream2 {
    let field_key = field.ident.to_string();

    let inner = quote! {
        match ::nebula_validator::validators::matches_regex(#pattern) {
            Ok(v) => {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                    errors.add(e.with_field(#field_key));
                }
            }
            Err(e) => {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(
                        "invalid_regex_pattern",
                        format!("invalid regex pattern `{}`: {}", #pattern, e),
                    )
                    .with_field(#field_key),
                );
            }
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

// ---------------------------------------------------------------------------
// Each-loop codegen
// ---------------------------------------------------------------------------

/// Generate the `for (index, value) in collection.iter().enumerate()` loop
/// for `each(...)` rules.
fn emit_each_loop(field: &FieldDef, each: &EachRules) -> TokenStream2 {
    let field_name = &field.ident;
    let field_key = field_name.to_string();

    let each_checks: Vec<TokenStream2> = each
        .rules
        .iter()
        .map(|rule| emit_each_rule(rule, field))
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
fn emit_each_rule(rule: &Rule, field: &FieldDef) -> TokenStream2 {
    let message = &field.message;

    match rule {
        Rule::MinLength(n) => emit_each_len_check(*n, true, message),
        Rule::MaxLength(n) => emit_each_len_check(*n, false, message),
        Rule::ExactLength(n) => emit_each_exact_len(*n, message),
        Rule::Min(bound) => emit_each_cmp_check(bound, true, message),
        Rule::Max(bound) => emit_each_cmp_check(bound, false, message),
        Rule::StringFormat(fmt) => emit_each_str_validator(string_format_to_tokens(*fmt), message),
        Rule::StringFactory { kind, arg } => {
            emit_each_str_validator(string_factory_to_tokens(*kind, arg), message)
        }
        Rule::Regex(pattern) => emit_each_regex(pattern, message),
        Rule::Nested => emit_each_nested(message),
        Rule::Custom(expr) => emit_each_custom(expr, message),
        // Other rules are not valid inside each(...) but parse phase would
        // not produce them; this arm is for completeness.
        _ => quote!(),
    }
}

// ---------------------------------------------------------------------------
// Each-loop per-rule helpers
// ---------------------------------------------------------------------------

/// Emit element-level min_length or max_length check.
fn emit_each_len_check(bound: usize, is_min: bool, message: &Option<String>) -> TokenStream2 {
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

    if let Some(msg) = message {
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
    }
}

/// Emit element-level exact_length check.
fn emit_each_exact_len(expected: usize, message: &Option<String>) -> TokenStream2 {
    if let Some(msg) = message {
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
    }
}

/// Emit element-level min or max numeric comparison check.
fn emit_each_cmp_check(
    bound: &TokenStream2,
    is_min: bool,
    message: &Option<String>,
) -> TokenStream2 {
    let code = if is_min { "min" } else { "max" };
    let cmp = if is_min {
        quote!(value < &#bound)
    } else {
        quote!(value > &#bound)
    };
    let op_str = if is_min { ">=" } else { "<=" };

    if let Some(msg) = message {
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
    }
}

/// Emit element-level string validator check.
fn emit_each_str_validator(
    validator_expr: TokenStream2,
    message: &Option<String>,
) -> TokenStream2 {
    if let Some(msg) = message {
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
    }
}

/// Emit element-level regex validator check.
fn emit_each_regex(pattern: &str, message: &Option<String>) -> TokenStream2 {
    if let Some(msg) = message {
        quote! {
            match ::nebula_validator::validators::matches_regex(#pattern) {
                Ok(v) => {
                    if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                        e = e.with_field(each_field.clone());
                        e.message = ::std::borrow::Cow::Owned(#msg.to_string());
                        errors.add(e);
                    }
                }
                Err(e) => {
                    let mut err = ::nebula_validator::foundation::ValidationError::new(
                        "invalid_regex_pattern",
                        format!("invalid regex pattern `{}`: {}", #pattern, e),
                    )
                    .with_field(each_field.clone());
                    err.message = ::std::borrow::Cow::Owned(#msg.to_string());
                    errors.add(err);
                }
            }
        }
    } else {
        quote! {
            match ::nebula_validator::validators::matches_regex(#pattern) {
                Ok(v) => {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                        errors.add(e.with_field(each_field.clone()));
                    }
                }
                Err(e) => {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::new(
                            "invalid_regex_pattern",
                            format!("invalid regex pattern `{}`: {}", #pattern, e),
                        )
                        .with_field(each_field.clone()),
                    );
                }
            }
        }
    }
}

/// Emit element-level nested validation check.
fn emit_each_nested(message: &Option<String>) -> TokenStream2 {
    if let Some(msg) = message {
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
    }
}

/// Emit element-level custom validator check.
fn emit_each_custom(expr: &TokenStream2, message: &Option<String>) -> TokenStream2 {
    if let Some(msg) = message {
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
    }
}

// ---------------------------------------------------------------------------
// StringFormat / StringFactory → TokenStream mapping
// ---------------------------------------------------------------------------

/// Convert a [`StringFormat`] variant to its validator constructor expression.
fn string_format_to_tokens(format: StringFormat) -> TokenStream2 {
    match format {
        StringFormat::NotEmpty => quote!(::nebula_validator::validators::not_empty()),
        StringFormat::Alphanumeric => quote!(::nebula_validator::validators::alphanumeric()),
        StringFormat::Alphabetic => quote!(::nebula_validator::validators::alphabetic()),
        StringFormat::Numeric => quote!(::nebula_validator::validators::numeric()),
        StringFormat::Lowercase => quote!(::nebula_validator::validators::lowercase()),
        StringFormat::Uppercase => quote!(::nebula_validator::validators::uppercase()),
        StringFormat::Email => quote!(::nebula_validator::validators::email()),
        StringFormat::Url => quote!(::nebula_validator::validators::url()),
        StringFormat::Ipv4 => quote!(::nebula_validator::validators::ipv4()),
        StringFormat::Ipv6 => quote!(::nebula_validator::validators::ipv6()),
        StringFormat::IpAddr => quote!(::nebula_validator::validators::ip_addr()),
        StringFormat::Hostname => quote!(::nebula_validator::validators::hostname()),
        StringFormat::Uuid => quote!(::nebula_validator::validators::uuid()),
        StringFormat::Date => quote!(::nebula_validator::validators::date()),
        StringFormat::DateTime => quote!(::nebula_validator::validators::date_time()),
        StringFormat::Time => quote!(::nebula_validator::validators::time()),
    }
}

/// Convert a [`StringFactoryKind`] and its argument to a validator constructor expression.
fn string_factory_to_tokens(kind: StringFactoryKind, arg: &str) -> TokenStream2 {
    match kind {
        StringFactoryKind::Contains => quote!(::nebula_validator::validators::contains(#arg)),
        StringFactoryKind::StartsWith => {
            quote!(::nebula_validator::validators::starts_with(#arg))
        }
        StringFactoryKind::EndsWith => quote!(::nebula_validator::validators::ends_with(#arg)),
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Extract the `Vec<T>` element type from a field's inner type.
///
/// Falls back to the inner type itself if extraction fails (should not happen
/// after parse-phase validation).
fn vec_inner_type_from_field(field: &FieldDef) -> &Type {
    vec_inner_type(&field.inner_ty).unwrap_or(&field.inner_ty)
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

/// Check if a type is `Option<T>`.
fn option_type_check(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Option")
}
