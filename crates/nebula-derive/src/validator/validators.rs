//! Validator generation utilities
//!
//! This module contains helper functions for generating validator code
//! from validation attributes. Each function handles a specific category
//! of validators (string, numeric, collection, etc.).

use super::parse::ValidationAttrs;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// Add string validators to the validators list
///
/// Handles: `min_length`, `max_length`, `exact_length`, email, url, regex,
/// alphanumeric, contains, `starts_with`, `ends_with`
pub(super) fn add_string_validators(validators: &mut Vec<TokenStream>, attrs: &ValidationAttrs) {
    // Length validators
    if let Some(val) = attrs.min_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::min_length(#val)
        });
    }
    if let Some(val) = attrs.max_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::max_length(#val)
        });
    }
    if let Some(val) = attrs.exact_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::exact_length(#val)
        });
    }

    // Format validators (no arguments)
    if attrs.email {
        validators.push(quote! {
            ::nebula_validator::validators::string::email()
        });
    }
    if attrs.url {
        validators.push(quote! {
            ::nebula_validator::validators::string::url()
        });
    }
    if attrs.alphanumeric {
        validators.push(quote! {
            ::nebula_validator::validators::string::alphanumeric()
        });
    }

    // Regex validator
    if let Some(pattern) = &attrs.regex {
        validators.push(quote! {
            ::nebula_validator::validators::string::matches_regex(#pattern)
                .expect("Invalid regex pattern")
        });
    }

    // Content validators
    if let Some(val) = &attrs.contains {
        validators.push(quote! {
            ::nebula_validator::validators::string::contains(#val)
        });
    }
    if let Some(val) = &attrs.starts_with {
        validators.push(quote! {
            ::nebula_validator::validators::string::starts_with(#val)
        });
    }
    if let Some(val) = &attrs.ends_with {
        validators.push(quote! {
            ::nebula_validator::validators::string::ends_with(#val)
        });
    }
}

/// Add text format validators (UUID, `DateTime`, JSON, etc.)
///
/// These validators use the builder pattern with `Validator::new()`
pub(super) fn add_text_validators(validators: &mut Vec<TokenStream>, attrs: &ValidationAttrs) {
    if attrs.uuid {
        validators.push(quote! {
            ::nebula_validator::validators::text::Uuid::new()
        });
    }
    if attrs.datetime {
        validators.push(quote! {
            ::nebula_validator::validators::text::DateTime::new()
        });
    }
    if attrs.json {
        validators.push(quote! {
            ::nebula_validator::validators::text::Json::new()
        });
    }
    if attrs.slug {
        validators.push(quote! {
            ::nebula_validator::validators::text::Slug::new()
        });
    }
    if attrs.hex {
        validators.push(quote! {
            ::nebula_validator::validators::text::Hex::new()
        });
    }
    if attrs.base64 {
        validators.push(quote! {
            ::nebula_validator::validators::text::Base64::new()
        });
    }
}

/// Add numeric validators based on attributes
///
/// Handles: min, max, range, positive, negative, even, odd
pub(super) fn add_numeric_validators(validators: &mut Vec<TokenStream>, attrs: &ValidationAttrs) {
    if let Some(val) = &attrs.min {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::min(#val)
        });
    }
    if let Some(val) = &attrs.max {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::max(#val)
        });
    }

    // Range validator (requires both min and max)
    if let (Some(min), Some(max)) = (&attrs.range_min, &attrs.range_max) {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::in_range(#min, #max)
        });
    }

    // Sign validators
    if attrs.positive {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::positive()
        });
    }
    if attrs.negative {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::negative()
        });
    }

    // Parity validators
    if attrs.even {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::even()
        });
    }
    if attrs.odd {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::odd()
        });
    }
}

/// Add collection validators based on attributes
///
/// Handles: `min_size`, `max_size`, unique, `non_empty`
pub(super) fn add_collection_validators(
    validators: &mut Vec<TokenStream>,
    attrs: &ValidationAttrs,
) {
    if let Some(val) = attrs.min_size {
        validators.push(quote! {
            ::nebula_validator::validators::collection::min_size(#val)
        });
    }
    if let Some(val) = attrs.max_size {
        validators.push(quote! {
            ::nebula_validator::validators::collection::max_size(#val)
        });
    }

    if attrs.unique {
        validators.push(quote! {
            ::nebula_validator::validators::collection::unique()
        });
    }
    if attrs.non_empty {
        validators.push(quote! {
            ::nebula_validator::validators::collection::non_empty()
        });
    }
}

/// Add logical validators based on attributes
///
/// Handles: required
pub(super) fn add_logical_validators(validators: &mut Vec<TokenStream>, attrs: &ValidationAttrs) {
    if attrs.required {
        validators.push(quote! {
            ::nebula_validator::validators::logical::required()
        });
    }
}

/// Add custom validator if specified
///
/// Parses the custom function name and adds it to the validators list
pub(super) fn add_custom_validator(
    validators: &mut Vec<TokenStream>,
    attrs: &ValidationAttrs,
) -> syn::Result<()> {
    if let Some(custom_fn) = &attrs.custom {
        let custom_ident = syn::parse_str::<Ident>(custom_fn)?;
        validators.push(quote! { #custom_ident });
    }
    Ok(())
}

/// Chain validators with `.and()` combinator
///
/// Takes a vector of validator `TokenStreams` and combines them:
/// - Single validator: returns it as-is
/// - Multiple validators: chains with `.and()`
///
/// # Errors
///
/// Returns error if validators list is empty
pub(super) fn chain_validators(validators: Vec<TokenStream>) -> syn::Result<TokenStream> {
    if validators.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "No validators specified for field",
        ));
    }

    if validators.len() == 1 {
        Ok(validators
            .into_iter()
            .next()
            .expect("validators vec has exactly 1 element"))
    } else {
        let mut iter = validators.into_iter();
        let first = iter.next().expect("validators vec has at least 2 elements");
        let rest: Vec<_> = iter.collect();
        Ok(quote! {
            #first #(.and(#rest))*
        })
    }
}
