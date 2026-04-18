//! Implementation of `#[derive(EnumSelect)]`.
//!
//! Generates `impl HasSelectOptions for T { fn select_options() -> Vec<SelectOption> { ... } }`
//! — one [`SelectOption`](nebula_schema::SelectOption) per unit variant. Only
//! unit-style enums are supported; the derive rejects variants that carry
//! payloads with a compile error.
//!
//! # Value encoding
//!
//! Each variant's stored value is `snake_case(variant_name)` — this is
//! **hardcoded** and does not read `serde` attributes. If an enum also
//! derives `Serialize` / `Deserialize` and intends the catalog options
//! to round-trip back into the enum, the enum author must pin
//! `#[serde(rename_all = "snake_case")]` (or `#[serde(rename = "...")]`
//! per variant) to match. Reading serde attrs here is tracked for a
//! follow-up — at this point in the lifecycle the only consumers of
//! `EnumSelect` are not also `Serialize`-derivers, so hardcoding the
//! safe default is the simplest honest implementation.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, Fields};

use crate::attrs::ParamAttrs;

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let crate_path = crate::crate_path();
    let ty_name = &input.ident;
    let generics = &input.generics;
    let (impl_g, ty_g, where_g) = generics.split_for_impl();

    let variants = match &input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => {
            return Err(syn::Error::new_spanned(
                ty_name,
                "#[derive(EnumSelect)] only supports enums",
            ));
        },
    };

    let mut option_exprs = Vec::with_capacity(variants.len());
    for variant in variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "#[derive(EnumSelect)] only supports unit variants (no payloads)",
            ));
        }
        let variant_name = &variant.ident;
        let value = snake_case(&variant_name.to_string());
        let param = ParamAttrs::from_attrs(&variant.attrs)?;
        let label = param.label.unwrap_or_else(|| variant_name.to_string());
        let description = param.description;
        let mut expr = quote! {
            #crate_path::SelectOption::new(::serde_json::Value::String(#value.to_owned()), #label)
        };
        if let Some(desc) = description {
            expr = quote! { #expr.with_description(#desc) };
        }
        option_exprs.push(expr);
    }

    Ok(quote! {
        impl #impl_g #crate_path::HasSelectOptions for #ty_name #ty_g #where_g {
            fn select_options() -> ::std::vec::Vec<#crate_path::SelectOption> {
                ::std::vec![
                    #( #option_exprs ),*
                ]
            }
        }
    })
}

/// Convert a `CamelCase` identifier to `snake_case`.
fn snake_case(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    let mut prev_lower = false;
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 && prev_lower {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    out
}
