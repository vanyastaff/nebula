//! Field-level credential slot detection for `#[derive(Resource)]` (Phase 4 / ADR-0044).
//!
//! Walks the struct fields and identifies `#[credential(...)]` attributes.
//! For each, the field type must follow one of:
//!
//! - `CredentialGuard<C>` — required + eager
//! - `Option<CredentialGuard<C>>` — optional + eager
//! - `Lazy<CredentialGuard<C>>` — required + lazy
//! - `Option<Lazy<CredentialGuard<C>>>` — optional + lazy
//!
//! Detection is by path-tail name (last `PathSegment::ident`) so the
//! macro accepts both bare `CredentialGuard<...>` and fully-qualified
//! `nebula_credential::CredentialGuard<...>`.
//!
//! Resources do not declare resource-typed slots — they ARE resources.
//! `#[resource]` field attributes are rejected with a clear error.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Field, Fields, GenericArgument, Ident, PathArguments, Result, Type};

/// One parsed credential slot field.
#[derive(Debug, Clone)]
pub(crate) struct ParsedCredentialSlot {
    /// Field identifier (and default slot key).
    pub field_ident: Ident,
    /// User-supplied `key = "..."` override, or `None` to default to field name.
    pub key_override: Option<String>,
    /// Optional `purpose = "..."` description (catalog/UI).
    #[allow(dead_code)]
    pub purpose: Option<String>,
    /// Whether the field is wrapped in `Option<...>`.
    pub optional: bool,
    /// Whether the field is wrapped in `Lazy<...>`.
    pub lazy: bool,
    /// The inner concrete credential type `C` underneath the wrappers.
    pub inner_type: Type,
}

impl ParsedCredentialSlot {
    /// The slot key — user-supplied `key = "..."` if present, else the field name.
    pub(crate) fn slot_key(&self) -> String {
        self.key_override
            .clone()
            .unwrap_or_else(|| self.field_ident.to_string())
    }
}

/// Walk the struct fields looking for `#[credential]` attrs.
///
/// Returns the parsed slot list. Returns an error on:
/// - `#[resource]` attribute on a field (resources don't declare resource slots)
/// - Slot attributes on field types that don't follow the recognised shape
/// - Duplicate slot keys
pub(crate) fn parse_credential_slot_fields(fields: &Fields) -> Result<Vec<ParsedCredentialSlot>> {
    let named = match fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                fields,
                "#[derive(Resource)] does not support tuple structs \
                 — use a named-field struct or a unit struct",
            ));
        },
        Fields::Unit => {
            return Ok(Vec::new());
        },
    };

    let mut out: Vec<ParsedCredentialSlot> = Vec::new();
    for field in named {
        if attrs::parse_attr_optional(&field.attrs, "resource")?.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "`#[resource]` slot attributes are not allowed on resource structs \
                 — resources cannot depend on other resources via slot binding. \
                 Use `#[credential(...)]` for credential dependencies.",
            ));
        }

        let credential_args = attrs::parse_attr_optional(&field.attrs, "credential")?;
        if let Some(args) = credential_args {
            let parsed = parse_one_slot(field, args)?;
            out.push(parsed);
        }
    }

    // Detect duplicate slot keys.
    for i in 0..out.len() {
        for j in (i + 1)..out.len() {
            let key_i = out[i].slot_key();
            let key_j = out[j].slot_key();
            if key_i == key_j {
                return Err(syn::Error::new_spanned(
                    &out[j].field_ident,
                    format!(
                        "duplicate credential slot key `{key_i}` on this field \
                         — same slot key is already declared on field `{}`",
                        out[i].field_ident,
                    ),
                ));
            }
        }
    }

    Ok(out)
}

fn parse_one_slot(field: &Field, args: attrs::AttrArgs) -> Result<ParsedCredentialSlot> {
    let field_ident = field
        .ident
        .clone()
        .expect("named field must have an ident; checked by parse_credential_slot_fields");

    let key_override = args.get_string("key");
    let purpose = args.get_string("purpose");

    let (optional, lazy, inner_type) = decode_field_type(&field.ty)?;

    Ok(ParsedCredentialSlot {
        field_ident,
        key_override,
        purpose,
        optional,
        lazy,
        inner_type,
    })
}

/// Decode the field type, recognising the four allowed shapes.
fn decode_field_type(ty: &Type) -> Result<(bool, bool, Type)> {
    let (optional, after_option) = if let Some(inner) = strip_path_tail(ty, "Option") {
        (true, inner)
    } else {
        (false, ty.clone())
    };

    let (lazy, after_lazy) = if let Some(inner) = strip_path_tail(&after_option, "Lazy") {
        (true, inner)
    } else {
        (false, after_option)
    };

    let Some(inner) = strip_path_tail(&after_lazy, "CredentialGuard") else {
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "field with `#[credential]` must have type `CredentialGuard<C>` \
                 (optionally wrapped in `Option<...>` and/or `Lazy<...>`) \
                 — got: {}",
                quote!(#ty),
            ),
        ));
    };

    Ok((optional, lazy, inner))
}

/// Match `Wrapper<Inner>` by path-tail (last segment ident == `wrapper_name`).
fn strip_path_tail(ty: &Type, wrapper_name: &str) -> Option<Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let last = type_path.path.segments.last()?;
    if last.ident != wrapper_name {
        return None;
    }
    let PathArguments::AngleBracketed(generic_args) = &last.arguments else {
        return None;
    };
    let first = generic_args.args.first()?;
    let GenericArgument::Type(inner) = first else {
        return None;
    };
    Some(inner.clone())
}

/// Generate the `Dependencies` registration calls for credential slot fields.
pub(crate) fn emit_slot_field_registrations(slots: &[ParsedCredentialSlot]) -> TokenStream2 {
    let calls: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let slot_key = slot.slot_key();
            let inner_ty = &slot.inner_type;
            let required = !slot.optional;
            let lazy = slot.lazy;
            quote! {
                .slot_field(::nebula_core::SlotField {
                    slot_key: #slot_key,
                    default_id: #slot_key,
                    kind: ::nebula_core::SlotKind::Credential {
                        type_id: ::std::any::TypeId::of::<#inner_ty>(),
                        type_name: ::std::any::type_name::<#inner_ty>(),
                        key: ::nebula_core::CredentialKey::new(
                            <#inner_ty as ::nebula_credential::Credential>::KEY,
                        )
                        .expect("credential KEY must be a valid CredentialKey"),
                    },
                    required: #required,
                    lazy: #lazy,
                })
            }
        })
        .collect();

    quote! { #(#calls)* }
}
