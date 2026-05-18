//! Field-level credential slot detection for `#[derive(Resource)]` (ADR-0044).
//!
//! Walks the struct fields and identifies `#[credential(...)]` attributes.
//! Each slot field is a [`SlotCell`] cell holding the resolved guard — the
//! framework swaps a rotated guard in through `&self` without `&mut` on the
//! resource, so the cell wrapper is mandatory.
//!
//! The only currently-accepted shape is `SlotCell<CredentialGuard<C>>`
//! (required + eager). `Option<…>`- and `Lazy<…>`-wrapped slots are
//! reserved for future optional/lazy binding but are currently rejected
//! at the derive site with a compile error, because the emitted accessor
//! only fits the plain cell shape (ADR-0044).
//!
//! Detection is by path-tail name (last `PathSegment::ident`) so the
//! macro accepts both bare `SlotCell<...>` / `CredentialGuard<...>` and
//! fully-qualified `nebula_resource::SlotCell<...>` /
//! `nebula_credential::CredentialGuard<...>`.
//!
//! [`SlotCell`]: nebula_resource::SlotCell
//!
//! Resources do not declare resource-typed slots — they ARE resources.
//! `#[resource]` field attributes are rejected with a clear error.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
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

/// Decode a `#[credential]` field type into `(optional, lazy, inner C)` per
/// the module-level shape table.
///
/// Layering, outermost first: an optional `Option<…>` (optional slot), the
/// mandatory `SlotCell<…>` cell, an optional `Lazy<…>` (lazy slot), then the
/// required `CredentialGuard<C>` carrying the inner concrete credential `C`.
fn decode_field_type(ty: &Type) -> Result<(bool, bool, Type)> {
    let (optional, after_option) = if let Some(inner) = strip_path_tail(ty, "Option") {
        (true, inner)
    } else {
        (false, ty.clone())
    };

    let Some(after_cell) = strip_path_tail(&after_option, "SlotCell") else {
        return Err(field_shape_error(ty));
    };

    let (lazy, after_lazy) = if let Some(inner) = strip_path_tail(&after_cell, "Lazy") {
        (true, inner)
    } else {
        (false, after_cell)
    };

    let Some(inner) = strip_path_tail(&after_lazy, "CredentialGuard") else {
        return Err(field_shape_error(ty));
    };

    // The generated accessor emits a single fixed body that only fits the
    // plain `SlotCell<CredentialGuard<C>>` shape; reject wrapper shapes at the
    // derive site until the accessor is generalized.
    if optional || lazy {
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "`#[credential]` slot must currently be exactly \
                 `SlotCell<CredentialGuard<C>>` — `Option<…>`- and `Lazy<…>`-wrapped \
                 slots are not yet supported by the generated accessor; got: {}",
                quote!(#ty),
            ),
        ));
    }

    Ok((optional, lazy, inner))
}

/// Diagnostic for a `#[credential]` field that does not match a recognised
/// slot-cell shape.
fn field_shape_error(ty: &Type) -> syn::Error {
    syn::Error::new_spanned(
        ty,
        format!(
            "field with `#[credential]` must have type `SlotCell<CredentialGuard<C>>` \
             (optionally wrapped in `Option<...>`, and/or with `Lazy<...>` between \
             the cell and the guard) — got: {}",
            quote!(#ty),
        ),
    )
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

/// Emit the body of `Resource::credential_slot_epoch` — an
/// **order-sensitive positional fold** over every declared
/// `#[credential]` `SlotCell` field's generation (ADR-0067 §Deferred
/// create-vs-rotate reconcile).
///
/// Derive-generated so a newly-added credential slot is automatically
/// folded into the epoch — an author cannot forget to include it (the
/// structural alternative to a "remember to update the epoch" comment).
/// With no slots the fold is empty and the epoch is `0` ("never bound"),
/// matching the trait default.
///
/// **Why a positional fold, not `max`.** The epoch's load-bearing
/// contract (#680) is "the value changes whenever *any* slot's
/// generation changes" — the resident create-vs-rotate reconcile
/// compares the epoch a runtime was built against with the live epoch
/// and only re-delivers the hook when they differ. `max` violates that:
/// a runtime built at `(slot_a=5, slot_b=10)` then rotated `slot_a→6`
/// still folds to `max=10`, so the reconcile would miss the stale
/// runtime entirely and silently report a rotation success while the
/// runtime keeps serving the pre-rotation credential. A position-weighted
/// fold `acc = acc * K + gen` (fixed odd `K`) changes on **every** slot
/// transition regardless of which slot moved or whether another slot's
/// generation happens to be larger. `wrapping_mul`/`wrapping_add` keep it
/// total (it is an opaque change-token, never compared by magnitude — the
/// reconcile only does `built != live`), and the per-slot
/// [`SlotCell::generation`](crate::SlotCell::generation) is itself
/// strictly monotone, so no real rotation sequence aliases back to a
/// prior epoch in practice.
pub(crate) fn emit_credential_slot_epoch_body(slots: &[ParsedCredentialSlot]) -> TokenStream2 {
    if slots.is_empty() {
        return quote! { 0 };
    }
    let gens: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let field = &slot.field_ident;
            quote! { self.#field.generation() }
        })
        .collect();
    // Position-weighted fold so EVERY slot transition changes the epoch
    // (not just the max-bearing one): `acc = acc * K + gen`. `K` is a
    // fixed odd constant (the 64-bit FNV-1a prime) for good dispersion;
    // wrapping arithmetic keeps the fold total — the epoch is an opaque
    // change-token compared only for equality by the create-vs-rotate
    // reconcile, never by magnitude. A single slot folds to
    // `0 * K + gen == gen` (unchanged from the prior single-slot
    // behaviour). Empty slot list returned `0` above ("never bound").
    quote! {
        {
            const __NEBULA_SLOT_EPOCH_K: u64 = 0x0000_0100_0000_01b3;
            [ #(#gens),* ]
                .into_iter()
                .fold(0u64, |acc, slot_gen| {
                    acc.wrapping_mul(__NEBULA_SLOT_EPOCH_K).wrapping_add(slot_gen)
                })
        }
    }
}

/// Per slot, emit a read accessor over the author-declared
/// `SlotCell<CredentialGuard<C>>` field.
///
/// A pure derive macro cannot add or rewrite struct fields, and
/// `ManagedResource` hands out `Arc<R>` (no `&mut R`). So the slot cell is
/// declared by the author; the framework populates and rotates it through
/// `&self` (`SlotCell::store`), and this accessor is the read side —
/// `self.<field>.load()` returns the current `Arc<CredentialGuard<C>>`, or
/// `None` until the framework binds it. No fields are added (ADR-0044).
pub(crate) fn emit_slot_accessors(slots: &[ParsedCredentialSlot]) -> TokenStream2 {
    let accessors: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let field = &slot.field_ident;
            let acc_ident = format_ident!("{}_slot", field);
            let inner = &slot.inner_type;
            quote! {
                #[doc = "Resolved credential for this slot, or `None` until the framework binds it."]
                pub fn #acc_ident(&self) -> ::std::option::Option<
                    ::std::sync::Arc<::nebula_credential::CredentialGuard<#inner>>
                > {
                    self.#field.load()
                }
            }
        })
        .collect();

    quote! { #(#accessors)* }
}
