//! Implementation of `#[derive(EnumSelect)]`.
//!
//! Generates `impl HasSelectOptions for T { fn select_options() -> Vec<SelectOption> { ... } }`
//! — one [`SelectOption`](nebula_schema::SelectOption) per unit variant. Only
//! unit-style enums are supported; the derive rejects variants that carry
//! payloads with a compile error.
//!
//! # Value encoding
//!
//! Each variant's stored value is its name in `snake_case`, computed with
//! [`heck`] so acronym runs split correctly (`HTTPProxy` → `http_proxy`, not
//! `httpproxy`). Two variants that collapse to the same value are a spanned
//! compile error rather than a silent collision.
//!
//! This does **not** yet read `serde` rename attributes. An enum that also
//! derives `Serialize` / `Deserialize` and wants its catalog options to
//! round-trip must keep an explicit `#[serde(rename_all = "snake_case")]` (or
//! per-variant `#[serde(rename = "...")]`) aligned with this default. Honoring
//! serde attributes is the keystone of the schema↔wire key-alignment follow-up.

use std::collections::HashMap;

use heck::ToSnakeCase;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, Fields, ext::IdentExt};

use crate::attrs::FieldAttrs;

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
    let mut seen_values = HashMap::with_capacity(variants.len());
    for variant in variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "#[derive(EnumSelect)] only supports unit variants (no payloads)",
            ));
        }
        let variant_name = &variant.ident;
        // Strip the raw-identifier prefix (`r#Type` → `Type`), then split on
        // case/acronym boundaries via `heck` so `HTTPProxy` → `http_proxy`.
        let value = variant_name.unraw().to_string().to_snake_case();
        if let Some(previous) = seen_values.insert(value.clone(), variant_name.clone()) {
            return Err(syn::Error::new_spanned(
                variant_name,
                format!(
                    "#[derive(EnumSelect)]: variants `{previous}` and `{variant_name}` both map to \
                     the option value `{value}` — rename one variant so its catalog value is unique",
                ),
            ));
        }
        let field_attr = FieldAttrs::from_attrs(&variant.attrs)?;
        let label = field_attr
            .label
            .unwrap_or_else(|| variant_name.unraw().to_string());
        let description = field_attr.description;
        let mut expr = quote! {
            #crate_path::SelectOption::new(
                #crate_path::__private::serde_json::Value::String(#value.to_owned()),
                #label,
            )
        };
        if let Some(desc) = description {
            expr = quote! { #expr.with_description(#desc) };
        }
        option_exprs.push(expr);
    }

    Ok(quote! {
        #[automatically_derived]
        impl #impl_g #crate_path::HasSelectOptions for #ty_name #ty_g #where_g {
            fn select_options() -> ::std::vec::Vec<#crate_path::SelectOption> {
                ::std::vec![
                    #( #option_exprs ),*
                ]
            }
        }
    })
}
