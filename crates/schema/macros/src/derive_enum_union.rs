//! Implementation of `#[derive(Schema)]` for payload-carrying enums.
//!
//! A Rust enum becomes a tagged-union schema ([`SchemaKind::Union`]): one
//! variant per enum variant, stored as the schema's sole root `Field::Mode`
//! (the marker design — see `nebula_schema::SchemaKind::Union`). The variant wire
//! keys and the recorded [`SerdeTagging`] reproduce serde's exact wire shape so
//! the schema variant key always equals the wire key (the C1 invariant the whole
//! serde-aware derive exists to protect).
//!
//! # Supported shapes
//!
//! - **Tagging:** external (serde default) and adjacent
//!   (`#[serde(tag = "..", content = "..")]`). Internally-tagged
//!   (`#[serde(tag = "..")]` alone) and untagged (`#[serde(untagged)]`) are
//!   **rejected** — the former inlines the tag into the payload's field namespace,
//!   the latter has no discriminant key, so neither can satisfy C1.
//! - **Variants:** unit (→ no payload), newtype `V(T)` where `T` derives
//!   [`HasSchema`] as a struct, and struct `V { .. }`. **Rejected:** tuple
//!   variants with more than one field, a newtype over a non-struct payload
//!   (primitive / `Vec` / `Option`), and `#[serde(flatten)]` on a variant field.
//! - **Renaming:** a variant's wire key follows serde — `#[serde(rename = "..")]`
//!   wins, else the container's `#[serde(rename_all = "..")]` applied to the
//!   variant, else the variant ident verbatim (serde's real default, *not*
//!   snake_case). A struct variant's field keys follow the **variant's** own
//!   `#[serde(rename_all)]` (serde does not cascade the container rule to
//!   struct-variant fields); the container `#[serde(rename_all_fields = ..)]` is
//!   rejected rather than silently ignored.

use std::collections::HashMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, Fields, Ident, Variant, ext::IdentExt};

use crate::{
    attrs::{FieldAttrs, RenameRule, SerdeAttrs, ValidateAttrs},
    derive_schema::{build_field_expr, resolve_field_key},
    type_infer::{FieldKind, classify},
};

/// Root key of the union's sole `Field::Mode`. Internal — *not* a wire key (the
/// wire discriminants are the variant keys); it only exists because every field
/// needs a `FieldKey`. A leading underscore is a valid key and signals "synthetic".
const UNION_ROOT_KEY: &str = "_nebula_union";

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let crate_path = crate::crate_path();
    let ty_name = &input.ident;
    let generics = &input.generics;
    let (impl_g, ty_g, where_g) = generics.split_for_impl();

    let Data::Enum(DataEnum { variants, .. }) = &input.data else {
        return Err(syn::Error::new_spanned(
            ty_name,
            "internal error: derive_enum_union expects an enum",
        ));
    };

    let container_serde = SerdeAttrs::from_attrs(&input.attrs)?;
    if let Some(span) = container_serde.rename_all_fields_span {
        return Err(syn::Error::new(
            span,
            "#[serde(rename_all_fields = ..)] is not yet honored by #[derive(Schema)] on enums; \
             put #[serde(rename_all = ..)] on each struct variant instead",
        ));
    }
    let tagging = classify_tagging(&container_serde, ty_name)?;

    let enum_name = ty_name.to_string();
    let mut variant_calls = Vec::with_capacity(variants.len());
    let mut seen_keys: HashMap<String, Ident> = HashMap::with_capacity(variants.len());
    for variant in variants {
        let variant_serde = SerdeAttrs::from_attrs(&variant.attrs)?;
        // `#[serde(skip)]` / `#[serde(skip_deserializing)]` mean the deserializer
        // never produces this variant, so it is not a *deserialization* arm. (The
        // union schema is a deserialization contract — see `SchemaKind::Union`. A
        // `skip_deserializing` variant can still *serialize*, but no value-layer
        // ingress validates serialized output against a union yet, so dropping it
        // here keeps the input contract honest.)
        if variant_serde.skip {
            continue;
        }
        // A `#[serde(alias = "..")]` on a variant is an extra wire key serde will
        // still deserialize into it — but the union records only one discriminant
        // per variant, so the alias would be a serde-accepted key the validator
        // rejects (a C1 desync). Reject it: pick one canonical wire key.
        if !variant_serde.aliases.is_empty() {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                format!(
                    "#[serde(alias = ..)] on the variant `{}` cannot be modeled by #[derive(Schema)]: \
                     a union records exactly one wire discriminant per variant, so the alias would be \
                     a key serde deserializes but schema validation rejects. Remove the alias, or make \
                     it the canonical key with #[serde(rename = ..)].",
                    variant.ident.unraw(),
                ),
            ));
        }
        let wire_key =
            resolve_variant_key(&variant.ident, &variant_serde, container_serde.rename_all);
        // Variant keys are used as schema path segments and as `Mode` variant keys
        // (which the runtime lint validates via `FieldKey::new`), so an invalid key
        // would panic the generated `schema()`. Reject it at expansion with a
        // single, variant-anchored message.
        if let Err(reason) = crate::validate_field_key(&wire_key) {
            return Err(syn::Error::new(
                variant.ident.span(),
                format!(
                    "variant `{}` resolves to the wire key `{wire_key}`, which is not a valid \
                     schema key ({reason}); rename the variant (or set `#[serde(rename = \"..\")]`) \
                     so its key is a non-empty ASCII identifier — variant keys are used as schema \
                     path segments",
                    variant.ident.unraw(),
                ),
            ));
        }
        if let Some(previous) = seen_keys.insert(wire_key.clone(), variant.ident.clone()) {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                format!(
                    "variants `{previous}` and `{}` both map to the union wire key `{wire_key}` — \
                     rename one so the discriminants are distinct",
                    variant.ident.unraw(),
                ),
            ));
        }

        let variant_attr = FieldAttrs::from_attrs(&variant.attrs)?;
        let label = variant_attr
            .label
            .unwrap_or_else(|| variant.ident.unraw().to_string());
        variant_calls.push(build_variant_call(
            variant,
            &wire_key,
            &label,
            &variant_serde,
            &enum_name,
            &crate_path,
        )?);
    }

    if variant_calls.is_empty() {
        return Err(syn::Error::new_spanned(
            ty_name,
            "#[derive(Schema)] on an enum needs at least one non-skipped variant",
        ));
    }

    let ty_name_str = ty_name.to_string();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_g #crate_path::HasSchema for #ty_name #ty_g #where_g {
            fn schema() -> #crate_path::ValidSchema {
                static __CACHE: ::std::sync::OnceLock<#crate_path::ValidSchema> =
                    ::std::sync::OnceLock::new();
                __CACHE
                    .get_or_init(|| {
                        // Build inside a `Result`-returning closure so a non-record
                        // newtype payload (`union_newtype_payload`) propagates via
                        // `?` into the single error/log path below — the failure
                        // surfaces here, not as a panic from library code.
                        let __build = || -> ::core::result::Result<
                            #crate_path::ValidSchema,
                            #crate_path::error::ValidationReport,
                        > {
                            let mut __mode = #crate_path::Field::mode(
                                #crate_path::FieldKey::new(#UNION_ROOT_KEY)
                                    .expect("union root key is a valid FieldKey"),
                            );
                            #( __mode = __mode #variant_calls; )*
                            #crate_path::ValidSchema::union(__mode, #tagging)
                        };
                        match __build() {
                            ::core::result::Result::Ok(s) => s,
                            ::core::result::Result::Err(report) => {
                                #crate_path::__private::tracing::error!(
                                    target: "nebula_schema::derive",
                                    type_name = #ty_name_str,
                                    report = ?report,
                                    "#[derive(Schema)] union schema-level lint failed at runtime"
                                );
                                ::core::panic!(
                                    "#[derive(Schema)] on enum `{}` produced an invalid union \
                                     schema — a variant's payload conflicts with a schema-level \
                                     lint. Report: {:?}",
                                    #ty_name_str,
                                    report,
                                );
                            },
                        }
                    })
                    .clone()
            }
        }
    })
}

/// Map the container's serde tagging attributes to a `SerdeTagging` constructor
/// token stream, rejecting the representations a faithful union schema cannot model.
fn classify_tagging(serde: &SerdeAttrs, ty_name: &Ident) -> syn::Result<TokenStream2> {
    let crate_path = crate::crate_path();
    if serde.untagged {
        return Err(syn::Error::new_spanned(
            ty_name,
            "#[serde(untagged)] enums are not supported by #[derive(Schema)] — an untagged union \
             has no discriminant key, so the schema cannot reproduce its wire shape",
        ));
    }
    match (&serde.tag, &serde.content) {
        (Some(tag), Some(content)) => Ok(quote! {
            #crate_path::SerdeTagging::Adjacent {
                tag: #tag.to_owned(),
                content: #content.to_owned(),
            }
        }),
        (Some(_), None) => Err(syn::Error::new_spanned(
            ty_name,
            "internally-tagged enums (`#[serde(tag = ..)]` without `content`) are not supported — \
             the tag inlines into the payload's field namespace; add `#[serde(content = ..)]` for \
             adjacent tagging",
        )),
        (None, Some(_)) => Err(syn::Error::new_spanned(
            ty_name,
            "#[serde(content = ..)] requires #[serde(tag = ..)]",
        )),
        (None, None) => Ok(quote! { #crate_path::SerdeTagging::External }),
    }
}

/// Resolve a variant's wire key, following serde exactly: an explicit
/// `#[serde(rename = "..")]` wins, else the container's `#[serde(rename_all)]`
/// applied to the variant via serde's variant algorithm, else the variant ident
/// **verbatim** (serde's real default for enum variants).
fn resolve_variant_key(
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
        None => base,
    }
}

/// Build the `.variant(..)` / `.variant_empty(..)` call for one enum variant.
fn build_variant_call(
    variant: &Variant,
    wire_key: &str,
    label: &str,
    variant_serde: &SerdeAttrs,
    enum_name: &str,
    crate_path: &TokenStream2,
) -> syn::Result<TokenStream2> {
    match &variant.fields {
        Fields::Unit => Ok(quote! { .variant_empty(#wire_key, #label) }),
        Fields::Unnamed(unnamed) => {
            let mut fields = unnamed.unnamed.iter();
            let (Some(only), None) = (fields.next(), fields.next()) else {
                return Err(syn::Error::new_spanned(
                    variant,
                    "tuple variants with more than one field are not supported by \
                     #[derive(Schema)]; use a struct variant `{ .. }` with named fields",
                ));
            };
            // A newtype variant `V(T)`: serde external emits `{ "V": <T> }`, so the
            // payload is `T`'s schema. `classify` is syntactic and cannot tell a
            // struct from an enum or `serde_json::Value`, so the macro accepts any
            // `UserDefined` type here and the runtime guard `union_newtype_payload`
            // rejects a non-record payload at `schema()` init (a union/`Any` payload
            // would otherwise splice keys serde never emits — a C1 break). Primitive
            // / `Vec` / `Option` payloads are rejected at expansion (no field shape).
            match classify(&only.ty) {
                FieldKind::UserDefined(ty) => {
                    let payload_ty = &*ty;
                    let payload = quote! {
                        #crate_path::__private::union_newtype_payload(
                            #crate_path::FieldKey::new(#wire_key)
                                .expect("variant wire key validated at macro expansion"),
                            <#payload_ty as #crate_path::HasSchema>::schema(),
                            #enum_name,
                            #wire_key,
                        )?
                    };
                    Ok(quote! { .variant(#wire_key, #label, #payload) })
                },
                _ => Err(syn::Error::new_spanned(
                    &only.ty,
                    "a newtype variant's payload must be a struct that derives `Schema`; \
                     primitive, `Vec`, and `Option` newtype payloads are not yet supported — \
                     wrap the value in a `#[derive(Schema)]` struct",
                )),
            }
        },
        Fields::Named(named) => {
            let mut field_exprs = Vec::with_capacity(named.named.len());
            for field in &named.named {
                let field_name = field.ident.as_ref().ok_or_else(|| {
                    syn::Error::new_spanned(field, "anonymous struct-variant field")
                })?;
                let field_serde = SerdeAttrs::from_attrs(&field.attrs)?;
                if let Some(flatten_span) = field_serde.flatten_span {
                    return Err(syn::Error::new(
                        flatten_span,
                        "#[serde(flatten)] is not supported inside a struct variant",
                    ));
                }
                let field_attr = FieldAttrs::from_attrs(&field.attrs)?;
                if field_attr.skip || field_serde.skip {
                    continue;
                }
                let validate = ValidateAttrs::from_attrs(&field.attrs)?;
                let kind = classify(&field.ty);
                // Struct-variant field keys follow the VARIANT's own rename_all
                // (serde does not cascade the container rule to them).
                let key_str =
                    resolve_field_key(field_name, &field_serde, variant_serde.rename_all)?;
                // Field-level `#[serde(alias)]` become read-aliases on the payload
                // field (deduped — serde tolerates a repeated alias but the runtime
                // alias lint rejects duplicates; each validated as a `FieldKey`).
                let mut field_read_aliases: Vec<String> =
                    Vec::with_capacity(field_serde.aliases.len());
                for alias in &field_serde.aliases {
                    if field_read_aliases.iter().any(|seen| seen == alias) {
                        continue;
                    }
                    crate::derive_schema::check_field_key(alias, field_name.span())?;
                    field_read_aliases.push(alias.clone());
                }
                field_exprs.push(build_field_expr(
                    field_name,
                    &key_str,
                    &kind,
                    &field_attr,
                    &validate,
                    &field_read_aliases,
                    crate_path,
                )?);
            }
            let payload = quote! {
                #crate_path::Field::object(
                    #crate_path::FieldKey::new(#wire_key)
                        .expect("variant wire key validated at macro expansion"),
                )
                #( .add(#field_exprs) )*
            };
            Ok(quote! { .variant(#wire_key, #label, #payload) })
        },
    }
}
