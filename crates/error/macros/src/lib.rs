//! # nebula-error-macros
//!
//! Proc-macros for the nebula-error crate.
//!
//! Provides the [`Classify`] derive macro that auto-generates the
//! `nebula_error::Classify` trait implementation from `#[classify(...)]`
//! attributes on enum variants.
//!
//! ## Example
//!
//! ```ignore
//! #[derive(Debug, thiserror::Error, Classify)]
//! enum MyError {
//!     #[classify(category = "timeout", code = "MY_TIMEOUT")]
//!     #[error("timed out")]
//!     Timeout,
//!
//!     #[classify(category = "validation", code = "MY_INVALID", severity = "warning")]
//!     #[error("invalid")]
//!     Invalid,
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

/// Derive the `Classify` trait for an error enum.
///
/// Place `#[classify(category = "...", code = "...")]` on each variant.
///
/// # Required attributes
///
/// - `category` — one of: `not_found`, `validation`, `authentication`, `authorization`, `conflict`,
///   `rate_limit`, `timeout`, `exhausted`, `cancelled`, `internal`, `external`, `unsupported`
/// - `code` — a string literal for the machine-readable error code
///
/// # Optional attributes
///
/// - `severity` — `"error"` (default), `"warning"`, or `"info"`
/// - `retryable` — `true` or `false` to override category default
/// - `retry_after_secs` — integer seconds for a `RetryHint`
///
/// # Errors
///
/// Compile-time errors are emitted when:
/// - The macro is applied to a non-enum type
/// - A variant is missing the `#[classify(...)]` attribute
/// - A required attribute (`category` or `code`) is missing
/// - An unknown category or severity string is used
#[proc_macro_derive(Classify, attributes(classify))]
pub fn derive_classify(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match classify_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed classification for a single variant.
struct VariantClassification {
    category: TokenStream2,
    code: String,
    severity: Option<TokenStream2>,
    retryable: Option<bool>,
    retry_after_secs: Option<u64>,
}

fn classify_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                enum_name,
                "Classify can only be derived for enums",
            ));
        }
    };

    let mut category_arms = Vec::new();
    let mut code_arms = Vec::new();
    let mut has_severity_override = false;
    let mut has_retryable_override = false;
    let mut has_retry_hint = false;
    let mut severity_arms = Vec::new();
    let mut retryable_arms = Vec::new();
    let mut retry_hint_arms = Vec::new();

    for variant in &data.variants {
        let variant_name = &variant.ident;
        let classification = parse_classify_attr(variant)?;
        let pattern = build_pattern(enum_name, variant_name, &variant.fields);

        let cat = &classification.category;
        category_arms.push(quote! { #pattern => #cat, });

        let code_str = &classification.code;
        code_arms.push(quote! {
            #pattern => ::nebula_error::ErrorCode::new(#code_str),
        });

        if let Some(ref sev) = classification.severity {
            has_severity_override = true;
            severity_arms.push(quote! { #pattern => #sev, });
        } else {
            severity_arms.push(quote! {
                #pattern => ::nebula_error::ErrorSeverity::Error,
            });
        }

        if let Some(retryable) = classification.retryable {
            has_retryable_override = true;
            retryable_arms.push(quote! { #pattern => #retryable, });
        } else {
            retryable_arms.push(quote! {
                #pattern => self.category().is_default_retryable(),
            });
        }

        if let Some(secs) = classification.retry_after_secs {
            has_retry_hint = true;
            retry_hint_arms.push(quote! {
                #pattern => ::core::option::Option::Some(
                    ::nebula_error::RetryHint::after(
                        ::core::time::Duration::from_secs(#secs)
                    )
                ),
            });
        } else {
            retry_hint_arms.push(quote! {
                #pattern => ::core::option::Option::None,
            });
        }
    }

    let severity_method = if has_severity_override {
        Some(quote! {
            fn severity(&self) -> ::nebula_error::ErrorSeverity {
                match self {
                    #(#severity_arms)*
                }
            }
        })
    } else {
        None
    };

    let retryable_method = if has_retryable_override {
        Some(quote! {
            fn is_retryable(&self) -> bool {
                match self {
                    #(#retryable_arms)*
                }
            }
        })
    } else {
        None
    };

    let retry_hint_method = if has_retry_hint {
        Some(quote! {
            fn retry_hint(&self) -> ::core::option::Option<::nebula_error::RetryHint> {
                match self {
                    #(#retry_hint_arms)*
                }
            }
        })
    } else {
        None
    };

    Ok(quote! {
        impl #impl_generics ::nebula_error::Classify for #enum_name #ty_generics
        #where_clause
        {
            fn category(&self) -> ::nebula_error::ErrorCategory {
                match self {
                    #(#category_arms)*
                }
            }

            fn code(&self) -> ::nebula_error::ErrorCode {
                match self {
                    #(#code_arms)*
                }
            }

            #severity_method
            #retryable_method
            #retry_hint_method
        }
    })
}

/// Build a match pattern for a variant, ignoring all fields.
fn build_pattern(enum_name: &Ident, variant_name: &Ident, fields: &Fields) -> TokenStream2 {
    match fields {
        Fields::Unit => quote! { #enum_name::#variant_name },
        Fields::Unnamed(_) => quote! { #enum_name::#variant_name(..) },
        Fields::Named(_) => quote! { #enum_name::#variant_name { .. } },
    }
}

/// Parse the `#[classify(...)]` attribute from a variant.
fn parse_classify_attr(variant: &syn::Variant) -> syn::Result<VariantClassification> {
    let mut found = None;

    for attr in &variant.attrs {
        if !attr.path().is_ident("classify") {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate #[classify(...)] attribute",
            ));
        }

        found = Some(parse_classify_meta(attr)?);
    }

    found.ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            format!(
                "variant `{}` is missing a #[classify(...)] attribute",
                variant.ident,
            ),
        )
    })
}

/// Map a category string to its `ErrorCategory` variant token stream.
fn category_from_str(s: &str, span: &dyn quote::ToTokens) -> syn::Result<TokenStream2> {
    let variant = match s {
        "not_found" => quote! { NotFound },
        "validation" => quote! { Validation },
        "authentication" => quote! { Authentication },
        "authorization" => quote! { Authorization },
        "conflict" => quote! { Conflict },
        "rate_limit" => quote! { RateLimit },
        "timeout" => quote! { Timeout },
        "exhausted" => quote! { Exhausted },
        "cancelled" => quote! { Cancelled },
        "internal" => quote! { Internal },
        "external" => quote! { External },
        "unsupported" => quote! { Unsupported },
        "unavailable" => quote! { Unavailable },
        "data_too_large" => quote! { DataTooLarge },
        _ => {
            return Err(syn::Error::new_spanned(
                span,
                format!(
                    "unknown category `{s}` — expected one of: not_found, validation, \
                     authentication, authorization, conflict, rate_limit, timeout, \
                     exhausted, cancelled, internal, external, unsupported, \
                     unavailable, data_too_large"
                ),
            ));
        }
    };
    Ok(quote! { ::nebula_error::ErrorCategory::#variant })
}

/// Map a severity string to its `ErrorSeverity` variant token stream.
fn severity_from_str(s: &str, span: &dyn quote::ToTokens) -> syn::Result<TokenStream2> {
    let variant = match s {
        "error" => quote! { Error },
        "warning" => quote! { Warning },
        "info" => quote! { Info },
        _ => {
            return Err(syn::Error::new_spanned(
                span,
                format!("unknown severity `{s}` — expected one of: error, warning, info"),
            ));
        }
    };
    Ok(quote! { ::nebula_error::ErrorSeverity::#variant })
}

/// Parse the inner contents of `#[classify(...)]`.
fn parse_classify_meta(attr: &syn::Attribute) -> syn::Result<VariantClassification> {
    let mut category: Option<TokenStream2> = None;
    let mut code: Option<String> = None;
    let mut severity: Option<TokenStream2> = None;
    let mut retryable: Option<bool> = None;
    let mut retry_after_secs: Option<u64> = None;

    attr.parse_nested_meta(|meta| {
        let ident = meta
            .path
            .get_ident()
            .ok_or_else(|| syn::Error::new_spanned(&meta.path, "expected an identifier"))?;

        let name = ident.to_string();
        match name.as_str() {
            "category" => {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                category = Some(category_from_str(&lit.value(), &lit)?);
                Ok(())
            }
            "code" => {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                code = Some(lit.value());
                Ok(())
            }
            "severity" => {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                severity = Some(severity_from_str(&lit.value(), &lit)?);
                Ok(())
            }
            "retryable" => {
                let value = meta.value()?;
                let lit: syn::LitBool = value.parse()?;
                retryable = Some(lit.value());
                Ok(())
            }
            "retry_after_secs" => {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                retry_after_secs = Some(lit.base10_parse()?);
                Ok(())
            }
            _ => Err(syn::Error::new_spanned(
                ident,
                format!(
                    "unknown classify attribute `{name}` — expected one of: \
                     category, code, severity, retryable, retry_after_secs"
                ),
            )),
        }
    })?;

    let category = category.ok_or_else(|| {
        syn::Error::new_spanned(attr, "missing required `category` in #[classify(...)]")
    })?;

    let code = code.ok_or_else(|| {
        syn::Error::new_spanned(attr, "missing required `code` in #[classify(...)]")
    })?;

    Ok(VariantClassification {
        category,
        code,
        severity,
        retryable,
        retry_after_secs,
    })
}
