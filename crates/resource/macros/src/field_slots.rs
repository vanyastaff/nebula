//! Field-level credential slot detection for `#[derive(Resource)]`.
//!
//! Walks the struct fields and identifies `#[credential(...)]` attributes.
//! Each slot field is a `SlotCell` cell holding the resolved guard — the
//! framework swaps a rotated guard in through `&self` without `&mut` on the
//! resource, so the cell wrapper is mandatory.
//!
//! ## Accepted field type shapes
//!
//! Detection is by path-tail name (last `PathSegment::ident`) so both bare and
//! fully-qualified paths work:
//!
//! | Accepted shape | Matches |
//! |---|---|
//! | `SlotCell<CredentialGuard<C>>` | bare or `nebula_resource::SlotCell<nebula_credential::CredentialGuard<C>>` |
//! | `CredentialSlot<C>` | bare or `nebula_resource::CredentialSlot<C>` (alias for the above) |
//!
//! All other types on a `#[credential]`-annotated field are rejected at
//! expansion time with a compile error naming both accepted shapes.
//!
//! ## Slot-key validation (expansion-time)
//!
//! The `key = "..."` literal is validated against the same rules as
//! `CredentialKey::new` (via [`is_valid_credential_key`]):
//!
//! - Non-empty
//! - Max 64 bytes (ASCII)
//! - Allowed bytes: `[0-9A-Za-z_\-.]`
//! - Last byte must be alphanumeric (not `_`, `-`, `.`)
//! - No consecutive identical separators (`__`, `--`, `..`)
//!
//! An invalid literal produces a compile error at the literal span.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Field, Fields, GenericArgument, Ident, LitStr, PathArguments, Result, Type};

/// One parsed credential slot field.
#[derive(Debug, Clone)]
pub(crate) struct ParsedCredentialSlot {
    /// Field identifier (and default slot key).
    pub field_ident: Ident,
    /// User-supplied `key = "..."` override, or `None` to default to field name.
    pub key_override: Option<String>,
    /// Optional `purpose = "..."` description (catalog/UI).
    pub purpose: Option<String>,
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

// ── Expansion-time CredentialKey validation ────────────────────────────────

/// Max key length that mirrors `CredentialDomain::MAX_LENGTH` (default 64).
const MAX_CREDENTIAL_KEY_LEN: usize = 64;

/// Returns `true` iff `s` passes exactly the same rules as
/// `CredentialKey::new` for the default `CredentialDomain` (which uses
/// `is_valid_key_default` from `domain-key`).
///
/// Rules (from `domain-key` validation source):
/// 1. Non-empty.
/// 2. Length ≤ `MAX_CREDENTIAL_KEY_LEN` (64).
/// 3. Every byte is ASCII alphanumeric, `_`, `-`, or `.`.
/// 4. Last byte must be ASCII alphanumeric (not a separator).
/// 5. No consecutive identical separators: `__`, `--`, `..`.
///
/// Note: uppercase ASCII is allowed by the domain (the key_type macro does
/// NOT force lowercase — normalization is opt-in per domain). The `credential_key!`
/// macro in nebula-core also accepts uppercase.
pub(crate) fn is_valid_credential_key(s: &str) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();

    if len == 0 || len > MAX_CREDENTIAL_KEY_LEN {
        return false;
    }

    let mut i = 0;
    while i < len {
        let b = bytes[i];
        // Allowed: ASCII alphanumeric, underscore, hyphen, dot.
        if !matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'-' | b'.') {
            return false;
        }
        // Reject consecutive identical separators.
        if i > 0 && matches!(b, b'_' | b'-' | b'.') && bytes[i - 1] == b {
            return false;
        }
        i += 1;
    }

    // Last byte must be alphanumeric.
    matches!(bytes[len - 1], b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z')
}

// ── Main parse entry points ────────────────────────────────────────────────

/// Walk the struct fields looking for `#[credential]` attrs — variant used by
/// `#[derive(Resource)]`.
///
/// Returns the parsed slot list. Returns an error on:
/// - `#[resource]` attribute on a field (resources don't declare resource slots)
/// - Slot attributes on field types that don't follow the recognised shape
/// - `key = "..."` literal that fails CredentialKey validation
/// - Duplicate slot keys
pub(crate) fn parse_credential_slot_fields_slots(
    fields: &Fields,
) -> Result<Vec<ParsedCredentialSlot>> {
    let named = match fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(unnamed) => {
            // Tuple structs are allowed as long as no field carries `#[credential]`.
            // If one does, point at that field for a clear error.
            for field in &unnamed.unnamed {
                if attrs::parse_attr_optional(&field.attrs, "credential")?.is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "#[derive(Resource)] does not support `#[credential]` on \
                         tuple-struct fields — use a named-field struct",
                    ));
                }
            }
            return Ok(Vec::new());
        },
        Fields::Unit => return Ok(Vec::new()),
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

        if let Some(args) = attrs::parse_attr_optional(&field.attrs, "credential")? {
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
        .expect("named field must have an ident; checked by parse_credential_slot_fields_slots");

    let key_override = match args.get_string("key") {
        None => None,
        Some(k) => {
            // Validate the key literal at expansion time against CredentialKey rules.
            if !is_valid_credential_key(&k) {
                // Find the LitStr for a precise span.
                if let Some(lit_str) = find_key_litstr(field) {
                    return Err(syn::Error::new_spanned(
                        lit_str,
                        format!(
                            "invalid credential slot key `{k}` — must be non-empty, \
                             ≤64 bytes, contain only [A-Za-z0-9_.-], not end with a \
                             separator, and have no consecutive identical separators"
                        ),
                    ));
                }
                return Err(syn::Error::new_spanned(
                    &field_ident,
                    format!(
                        "invalid credential slot key `{k}` — must be non-empty, \
                         ≤64 bytes, contain only [A-Za-z0-9_.-], not end with a \
                         separator, and have no consecutive identical separators"
                    ),
                ));
            }
            Some(k)
        },
    };

    let purpose = args.get_string("purpose");

    let inner_type = decode_field_type_slots(&field.ty)?;

    Ok(ParsedCredentialSlot {
        field_ident,
        key_override,
        purpose,
        inner_type,
    })
}

/// Try to extract the `LitStr` from `#[credential(key = "...")]` for precise error spans.
fn find_key_litstr(field: &Field) -> Option<LitStr> {
    use syn::{Lit, Meta};
    for attr in &field.attrs {
        if !attr.path().is_ident("credential") {
            continue;
        }
        let Meta::List(list) = &attr.meta else {
            continue;
        };
        // Parse as nested metas to find `key = "..."`.
        let mut found: Option<LitStr> = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(ls) = lit {
                    found = Some(ls);
                }
            }
            Ok(())
        });
        if found.is_some() {
            return found;
        }
        // Fallback: the tokens span covers the list.
        let _ = list; // suppress unused warning
    }
    None
}

/// Decode a `#[credential]` field type for the `Resource` derive.
///
/// Accepted shapes (path-tail detection — bare or fully qualified):
/// - `SlotCell<CredentialGuard<C>>`
/// - `CredentialSlot<C>` (alias for the above)
///
/// Returns the inner credential type `C`.
fn decode_field_type_slots(ty: &Type) -> Result<Type> {
    // Shape 1: CredentialSlot<C>
    if let Some(inner) = strip_path_tail(ty, "CredentialSlot") {
        return Ok(inner);
    }

    // Shape 2: SlotCell<CredentialGuard<C>>
    if let Some(after_cell) = strip_path_tail(ty, "SlotCell")
        && let Some(inner) = strip_path_tail(&after_cell, "CredentialGuard")
    {
        return Ok(inner);
    }

    Err(syn::Error::new_spanned(
        ty,
        format!(
            "field with `#[credential]` must have type `SlotCell<CredentialGuard<C>>` \
             or `CredentialSlot<C>` — got: {ty}",
            ty = quote!(#ty),
        ),
    ))
}

// ── Shared path-tail stripper ──────────────────────────────────────────────

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

// ── Code emitters ──────────────────────────────────────────────────────────

/// Generate the `Dependencies` registration calls for credential slot fields,
/// wiring `purpose` when present.
pub(crate) fn emit_slot_field_registrations_with_purpose(
    slots: &[ParsedCredentialSlot],
) -> TokenStream2 {
    let calls: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let slot_key = slot.slot_key();
            let inner_ty = &slot.inner_type;
            // `required = true`: slot-less structs get no calls, all
            // named slots are required (optional slots not yet supported).
            let required = true;
            let lazy = false;
            let purpose_tokens = if let Some(p) = &slot.purpose {
                quote! { ::core::option::Option::Some(#p) }
            } else {
                quote! { ::core::option::Option::None }
            };
            quote! {
                .slot_field(::nebula_core::SlotField {
                    slot_key: #slot_key,
                    default_id: #slot_key,
                    kind: ::nebula_core::SlotKind::Credential {
                        type_id: ::std::any::TypeId::of::<#inner_ty>(),
                        type_name: ::std::any::type_name::<#inner_ty>(),
                        // The key literal was validated at expansion time —
                        // `is_valid_credential_key` guarantees this `.expect` is unreachable.
                        key: ::nebula_core::CredentialKey::new(
                            <#inner_ty as ::nebula_credential::Credential>::KEY,
                        )
                        .expect("Credential::KEY must satisfy CredentialKey rules; \
                                 fix the KEY constant on this Credential impl"),
                    },
                    required: #required,
                    lazy: #lazy,
                    purpose: #purpose_tokens,
                })
            }
        })
        .collect();

    quote! { #(#calls)* }
}

/// Emit the body of `HasCredentialSlots::credential_slot_epoch` — an
/// **order-sensitive positional fold** over every declared `#[credential]`
/// `SlotCell` field's generation (per-resource revoke deferral
/// create-vs-rotate reconcile).
///
/// With no slots the fold is empty and the epoch is `0` ("never bound").
///
/// **Why a positional fold, not `max`.** `max` violates the contract: a
/// runtime built at `(slot_a=5, slot_b=10)` then rotated `slot_a→6` still
/// folds to `max=10`, so the reconcile would miss the stale runtime and
/// silently report a rotation success while the runtime keeps serving the
/// pre-rotation credential. A position-weighted fold
/// `acc = acc * K + gen` (fixed odd `K`) changes on **every** slot
/// transition regardless of which slot moved or whether another slot's
/// generation happens to be larger.
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
    // Position-weighted fold: `acc = acc * K + gen`. `K` is the 64-bit
    // FNV-1a prime for good dispersion; wrapping arithmetic keeps it total.
    // A single slot folds to `0 * K + gen == gen`. The epoch is an opaque
    // change-token compared only for equality by the reconcile, never by magnitude.
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
/// `SlotCell<CredentialGuard<C>>` (or `CredentialSlot<C>`) field.
///
/// Returns an empty `TokenStream2` when `slots` is empty — the caller skips
/// emitting an empty `impl` block.
pub(crate) fn emit_slot_accessors(slots: &[ParsedCredentialSlot]) -> TokenStream2 {
    if slots.is_empty() {
        return quote! {};
    }
    let accessors: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let field = &slot.field_ident;
            let acc_ident = format_ident!("{}_slot", field);
            let inner = &slot.inner_type;
            quote! {
                /// Resolved credential for this slot, or `None` until the framework binds it.
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

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::is_valid_credential_key;

    /// The local validator must accept/reject exactly what `CredentialKey::new`
    /// does over a representative corpus.
    #[test]
    fn validator_matches_credential_key_new() {
        use nebula_core::CredentialKey;

        let cases: &[(&str, bool)] = &[
            // Valid
            ("my_api_key", true),
            ("db_auth", true),
            ("slot.a", true),
            ("slot-b", true),
            ("abc123", true),
            ("a", true),
            ("A", true),
            ("FooBar", true),
            ("epochfold.fake", true),
            ("_foo", true),
            ("foo_bar_baz", true),
            ("s1.s2-s3_s4", true),
            // Invalid: empty
            ("", false),
            // Invalid: trailing separator
            ("foo_", false),
            ("foo-", false),
            ("foo.", false),
            // Invalid: consecutive separators
            ("a__b", false),
            ("a--b", false),
            ("a..b", false),
            // Invalid: spaces
            ("has space", false),
            ("has\ttab", false),
            // Invalid: unicode
            ("héllo", false),
            // Invalid: special chars
            ("a@b", false),
            ("a!b", false),
            // Leading digit is allowed
            ("1slot", true),
        ];

        for (key, expected) in cases {
            let runtime_result = CredentialKey::new(key).is_ok();
            let macro_result = is_valid_credential_key(key);
            assert_eq!(
                macro_result, *expected,
                "is_valid_credential_key({key:?}) = {macro_result}, expected {expected}"
            );
            assert_eq!(
                runtime_result, *expected,
                "CredentialKey::new({key:?}).is_ok() = {runtime_result}, expected {expected}"
            );
        }
    }
}
