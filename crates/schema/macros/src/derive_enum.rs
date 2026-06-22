//! Implementation of `#[derive(EnumSelect)]`.
//!
//! Generates `impl HasSelectOptions for T { fn select_options() -> Vec<SelectOption> { ... } }`
//! — one [`SelectOption`](nebula_schema::SelectOption) per unit variant. Only
//! unit-style enums are supported; the derive rejects variants that carry
//! payloads with a compile error.
//!
//! # Value encoding
//!
//! A variant's catalog value follows serde so options round-trip back into the
//! enum: an explicit `#[serde(rename = "..")]` wins, otherwise the enum's
//! `#[serde(rename_all = ..)]` is applied via serde's *exact* variant algorithm
//! (so `rename_all = "snake_case"` turns `HTTPProxy` into `h_t_t_p_proxy`, as
//! serde does). A variant marked `#[serde(skip)]` is omitted; two variants that
//! collapse to the same value are a spanned compile error.
//!
//! With no `#[serde(rename_all)]` the value defaults to the variant in
//! `snake_case` via [`heck`] (`HTTPProxy` → `http_proxy`) — an `EnumSelect` UI
//! convention, not serde's own default (which is the variant name as-is). Set
//! `#[serde(rename_all = "snake_case")]` to align catalog values with serde's
//! wire names exactly.

use std::collections::HashMap;

use heck::ToSnakeCase;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, Fields, Ident, ext::IdentExt};

use crate::attrs::{FieldAttrs, RenameRule, SerdeAttrs};

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

    let container_serde = SerdeAttrs::from_attrs(&input.attrs)?;
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
        let serde = SerdeAttrs::from_attrs(&variant.attrs)?;
        // A variant serde skips can never round-trip, so it is not a catalog option.
        if serde.skip {
            continue;
        }
        let value = resolve_variant_value(variant_name, &serde, container_serde.rename_all);
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

/// Resolve an enum variant's catalog (option) value, honoring serde: an explicit
/// `#[serde(rename = "..")]` wins, otherwise `#[serde(rename_all = ..)]` is
/// applied to the raw-stripped variant, otherwise the variant in `snake_case`
/// (the `EnumSelect` convention — this matches serde only when the enum also sets
/// `#[serde(rename_all = "snake_case")]`).
fn resolve_variant_value(
    variant_name: &Ident,
    serde: &SerdeAttrs,
    container_rename_all: Option<RenameRule>,
) -> String {
    if let Some(rename) = &serde.rename {
        return rename.clone();
    }
    let base = variant_name.unraw().to_string();
    match container_rename_all {
        Some(rule) => rule.apply_to_variant(&base),
        None => base.to_snake_case(),
    }
}
