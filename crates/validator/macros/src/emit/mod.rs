//! Emit phase: generates `TokenStream` from the [`ValidatorInput`] IR.
//!
//! The core architectural win: Option-wrapping and message-override are each
//! handled in ONE function instead of being duplicated across every rule.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Type;

use crate::model::{FieldDef, StringFactoryKind, StringFormat, ValidatorInput};

mod each;
mod rules;

use each::emit_each_loop;
use rules::emit_field_rule;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate the full `Validate`, `SelfValidating`, and `validate_fields()`
/// implementations from the parsed IR.
pub(crate) fn emit(input: &ValidatorInput) -> TokenStream2 {
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
pub(super) fn wrap_option(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
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
pub(super) fn wrap_message(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
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
// StringFormat / StringFactory → TokenStream mapping
// ---------------------------------------------------------------------------

/// Convert a [`StringFormat`] variant to its validator constructor expression.
pub(super) fn string_format_to_tokens(format: StringFormat) -> TokenStream2 {
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
pub(super) fn string_factory_to_tokens(kind: StringFactoryKind, arg: &str) -> TokenStream2 {
    match kind {
        StringFactoryKind::Contains => quote!(::nebula_validator::validators::contains(#arg)),
        StringFactoryKind::StartsWith => {
            quote!(::nebula_validator::validators::starts_with(#arg))
        },
        StringFactoryKind::EndsWith => quote!(::nebula_validator::validators::ends_with(#arg)),
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

use crate::types::vec_inner_type;

/// Extract the `Vec<T>` element type from a field's inner type.
///
/// Falls back to the inner type itself if extraction fails (should not happen
/// after parse-phase validation).
pub(super) fn vec_inner_type_from_field(field: &FieldDef) -> &Type {
    vec_inner_type(&field.inner_ty).unwrap_or(&field.inner_ty)
}
