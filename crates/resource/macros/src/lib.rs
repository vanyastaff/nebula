//! # nebula-resource-macros
//!
//! Proc-macros for the [`nebula-resource`] crate.
//!
//! Provides the [`ClassifyError`] derive macro that auto-generates
//! `From<UserError> for nebula_resource::Error` based on `#[classify(...)]`
//! attributes on enum variants.
//!
//! ## Example
//!
//! ```ignore
//! #[derive(Debug, thiserror::Error, ClassifyError)]
//! pub enum PgError {
//!     #[error("auth failed")]
//!     #[classify(permanent)]
//!     Auth(String),
//!
//!     #[error("connection failed")]
//!     #[classify(transient)]
//!     Connect(#[from] std::io::Error),
//!
//!     #[error("rate limited")]
//!     #[classify(exhausted, retry_after = "30s")]
//!     RateLimit,
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

mod resource;

/// Derive macro for the `Resource` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[resource(...)]` on the struct)
///
/// - `id = "..."` - Unique resource identifier (required)
/// - `config = Type` - Associated config type (required)
/// - `instance = Type` - Associated instance type (default: Self)
///
/// # Example
///
/// ```ignore
/// #[derive(Resource)]
/// #[resource(
///     id = "postgres",
///     config = PgConfig,
///     instance = PgPool
/// )]
/// pub struct PostgresResource;
/// ```
#[proc_macro_derive(Resource, attributes(resource))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    resource::derive(input)
}

/// Derive macro that generates `From<T> for nebula_resource::Error`.
///
/// Place `#[classify(kind)]` on each enum variant to specify how the
/// framework should handle errors of that variant.
///
/// # Supported kinds
///
/// - `transient` — retry with backoff
/// - `permanent` — never retry
/// - `exhausted` — retry after cooldown (optionally with `retry_after = "30s"`)
/// - `backpressure` — caller decides
/// - `cancelled` — operation was cancelled
///
/// # Errors
///
/// Compile-time errors are emitted when:
/// - The macro is applied to a non-enum type
/// - A variant is missing the `#[classify(...)]` attribute
/// - An unknown classification kind is used
/// - The `retry_after` duration string cannot be parsed
#[proc_macro_derive(ClassifyError, attributes(classify))]
pub fn derive_classify_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match classify_error_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed classification for a single variant.
struct Classification {
    kind: ClassifyKind,
}

enum ClassifyKind {
    Transient,
    Permanent,
    Exhausted {
        retry_after: Option<std::time::Duration>,
    },
    Backpressure,
    Cancelled,
}

fn classify_error_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                enum_name,
                "ClassifyError can only be derived for enums",
            ));
        }
    };

    let mut match_arms = Vec::new();

    for variant in &data.variants {
        let variant_name = &variant.ident;
        let classification = parse_classify_attr(variant)?;
        let pattern = build_pattern(enum_name, variant_name, &variant.fields);
        let constructor = build_constructor(&classification);

        match_arms.push(quote! {
            #pattern => #constructor,
        });
    }

    Ok(quote! {
        impl #impl_generics ::core::convert::From<#enum_name #ty_generics> for nebula_resource::Error
        #where_clause
        {
            fn from(err: #enum_name #ty_generics) -> Self {
                // Use Display to get the message before moving into the match.
                let __msg = ::std::string::ToString::to_string(&err);
                match err {
                    #(#match_arms)*
                }
            }
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

/// Build the `nebula_resource::Error::*` constructor call for a classification.
fn build_constructor(classification: &Classification) -> TokenStream2 {
    match &classification.kind {
        ClassifyKind::Transient => {
            quote! { nebula_resource::Error::transient(__msg) }
        }
        ClassifyKind::Permanent => {
            quote! { nebula_resource::Error::permanent(__msg) }
        }
        ClassifyKind::Exhausted { retry_after } => match retry_after {
            Some(dur) => {
                let secs = dur.as_secs();
                let nanos = dur.subsec_nanos();
                quote! {
                    nebula_resource::Error::exhausted(
                        __msg,
                        ::core::option::Option::Some(
                            ::core::time::Duration::new(#secs, #nanos)
                        ),
                    )
                }
            }
            None => {
                quote! {
                    nebula_resource::Error::exhausted(
                        __msg,
                        ::core::option::Option::None,
                    )
                }
            }
        },
        ClassifyKind::Backpressure => {
            quote! { nebula_resource::Error::backpressure(__msg) }
        }
        ClassifyKind::Cancelled => {
            quote! { nebula_resource::Error::cancelled() }
        }
    }
}

/// Parse the `#[classify(...)]` attribute from a variant.
fn parse_classify_attr(variant: &syn::Variant) -> syn::Result<Classification> {
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
                variant.ident
            ),
        )
    })
}

/// Parse the inner contents of `#[classify(...)]`.
fn parse_classify_meta(attr: &syn::Attribute) -> syn::Result<Classification> {
    let mut kind_ident: Option<Ident> = None;
    let mut retry_after: Option<std::time::Duration> = None;

    attr.parse_nested_meta(|meta| {
        let ident = meta
            .path
            .get_ident()
            .ok_or_else(|| syn::Error::new_spanned(&meta.path, "expected an identifier"))?;

        let name = ident.to_string();
        match name.as_str() {
            "transient" | "permanent" | "exhausted" | "backpressure" | "cancelled" => {
                if kind_ident.is_some() {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "multiple classification kinds specified",
                    ));
                }
                kind_ident = Some(ident.clone());
                Ok(())
            }
            "retry_after" => {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                let dur = parse_duration(&lit.value())
                    .map_err(|msg| syn::Error::new_spanned(&lit, msg))?;
                retry_after = Some(dur);
                Ok(())
            }
            _ => Err(syn::Error::new_spanned(
                ident,
                format!("unknown classify attribute `{name}`"),
            )),
        }
    })?;

    let kind_ident = kind_ident.ok_or_else(|| {
        syn::Error::new_spanned(attr, "missing classification kind (transient, permanent, exhausted, backpressure, cancelled)")
    })?;

    if retry_after.is_some() && kind_ident != "exhausted" {
        return Err(syn::Error::new_spanned(
            &kind_ident,
            "retry_after is only valid with `exhausted`",
        ));
    }

    let kind = match kind_ident.to_string().as_str() {
        "transient" => ClassifyKind::Transient,
        "permanent" => ClassifyKind::Permanent,
        "exhausted" => ClassifyKind::Exhausted { retry_after },
        "backpressure" => ClassifyKind::Backpressure,
        "cancelled" => ClassifyKind::Cancelled,
        _ => unreachable!(),
    };

    Ok(Classification { kind })
}

/// Parse a human-readable duration string like `"30s"`, `"5m"`, `"1h"`.
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours), `ms` (milliseconds).
/// Plain numbers without a suffix are treated as seconds.
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    if let Some(val) = s.strip_suffix("ms") {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid millisecond value: `{val}`"))?;
        return Ok(std::time::Duration::from_millis(n));
    }

    if let Some(val) = s.strip_suffix('s') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid second value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n));
    }

    if let Some(val) = s.strip_suffix('m') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid minute value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n * 60));
    }

    if let Some(val) = s.strip_suffix('h') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid hour value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n * 3600));
    }

    // Bare number = seconds
    let n: u64 = s
        .parse()
        .map_err(|_| format!("invalid duration: `{s}` (use a suffix: s, m, h, ms)"))?;
    Ok(std::time::Duration::from_secs(n))
}
