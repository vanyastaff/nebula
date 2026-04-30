//! Field-level slot detection for `#[derive(Action)]` Variant A.
//!
//! Walks the struct fields and identifies `#[resource(...)]` or
//! `#[credential(...)]` attributes. For each, the field type must follow
//! one of:
//!
//! - `ResourceGuard<R>` / `CredentialGuard<C>` — required + eager
//! - `Option<ResourceGuard<R>>` / `Option<CredentialGuard<C>>` — optional + eager
//! - `Lazy<ResourceGuard<R>>` / `Lazy<CredentialGuard<C>>` — required + lazy
//! - `Option<Lazy<ResourceGuard<R>>>` / `Option<Lazy<CredentialGuard<C>>>` — optional + lazy
//!
//! Detection is by path-tail name (last `PathSegment::ident`) so the
//! macro accepts both bare `ResourceGuard<...>` and fully-qualified
//! `nebula_resource::ResourceGuard<...>`.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Field, Fields, GenericArgument, Ident, PathArguments, Result, Type};

/// Slot kind detected on a field — resource or credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotKind {
    Resource,
    Credential,
}

/// One parsed slot field.
#[derive(Debug, Clone)]
pub(crate) struct ParsedSlotField {
    /// Field identifier (and default slot key).
    pub field_ident: Ident,
    /// User-supplied `key = "..."` override, or `None` to default to field name.
    pub key_override: Option<String>,
    /// Slot kind — resource or credential.
    pub kind: SlotKind,
    /// Whether the field is wrapped in `Option<...>`.
    pub optional: bool,
    /// Whether the field is wrapped in `Lazy<...>`.
    pub lazy: bool,
    /// The inner concrete type (`R` for resource, `C` for credential).
    pub inner_type: Type,
}

/// Walk the struct fields looking for `#[resource]` / `#[credential]` attrs.
///
/// Returns the parsed slot list. Returns an error on:
/// - Both `#[resource]` and `#[credential]` on the same field
/// - Slot attributes on field types that don't follow the recognised shape
/// - Duplicate slot keys
pub(crate) fn parse_slot_fields(fields: &Fields) -> Result<Vec<ParsedSlotField>> {
    let named = match fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                fields,
                "#[derive(Action)] does not support tuple structs \
                 — use a named-field struct or a unit struct",
            ));
        },
        Fields::Unit => {
            return Ok(Vec::new());
        },
    };

    let mut out: Vec<ParsedSlotField> = Vec::new();
    for field in named {
        let resource_args = attrs::parse_attr_optional(&field.attrs, "resource")?;
        let credential_args = attrs::parse_attr_optional(&field.attrs, "credential")?;

        match (resource_args, credential_args) {
            (None, None) => continue,
            (Some(_), Some(_)) => {
                return Err(syn::Error::new_spanned(
                    field,
                    "field has both `#[resource]` and `#[credential]` attributes \
                     — only one slot kind per field is allowed",
                ));
            },
            (Some(args), None) => {
                let parsed = parse_one_slot(field, args, SlotKind::Resource)?;
                out.push(parsed);
            },
            (None, Some(args)) => {
                let parsed = parse_one_slot(field, args, SlotKind::Credential)?;
                out.push(parsed);
            },
        }
    }

    // Detect duplicate slot keys — the registry compares slot keys, so
    // two fields with the same `key = "..."` or default-name collision
    // is a hard error.
    for i in 0..out.len() {
        for j in (i + 1)..out.len() {
            let key_i = out[i].slot_key();
            let key_j = out[j].slot_key();
            if key_i == key_j {
                return Err(syn::Error::new_spanned(
                    &out[j].field_ident,
                    format!(
                        "duplicate slot key `{key_i}` on this field \
                         — same slot key is already declared on field `{}`",
                        out[i].field_ident,
                    ),
                ));
            }
        }
    }

    Ok(out)
}

fn parse_one_slot(field: &Field, args: attrs::AttrArgs, kind: SlotKind) -> Result<ParsedSlotField> {
    let field_ident = field
        .ident
        .clone()
        .expect("named field must have an ident; checked by parse_slot_fields");

    let key_override = args.get_string("key");

    let (optional, lazy, inner_type) = decode_field_type(&field.ty, kind)?;

    Ok(ParsedSlotField {
        field_ident,
        key_override,
        kind,
        optional,
        lazy,
        inner_type,
    })
}

impl ParsedSlotField {
    /// The slot key — user-supplied `key = "..."` if present, else the field name.
    pub(crate) fn slot_key(&self) -> String {
        self.key_override
            .clone()
            .unwrap_or_else(|| self.field_ident.to_string())
    }

    /// The slot kind name for diagnostics: "resource" or "credential".
    pub(crate) fn kind_word(&self) -> &'static str {
        match self.kind {
            SlotKind::Resource => "resource",
            SlotKind::Credential => "credential",
        }
    }
}

/// Decode the field type, recognising the four allowed shapes.
///
/// Returns `(optional, lazy, inner)` where `inner` is the concrete `R` or
/// `C` underneath the wrappers.
fn decode_field_type(ty: &Type, kind: SlotKind) -> Result<(bool, bool, Type)> {
    let guard_ident = match kind {
        SlotKind::Resource => "ResourceGuard",
        SlotKind::Credential => "CredentialGuard",
    };

    // Strip Option<...>?
    let (optional, after_option) = if let Some(inner) = strip_path_tail(ty, "Option") {
        (true, inner)
    } else {
        (false, ty.clone())
    };

    // Strip Lazy<...>?
    let (lazy, after_lazy) = if let Some(inner) = strip_path_tail(&after_option, "Lazy") {
        (true, inner)
    } else {
        (false, after_option)
    };

    // The remaining type must be ResourceGuard<R> / CredentialGuard<C>.
    let Some(inner) = strip_path_tail(&after_lazy, guard_ident) else {
        let kw = match kind {
            SlotKind::Resource => "resource",
            SlotKind::Credential => "credential",
        };
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "field with `#[{kw}]` must have type `{guard_ident}<T>` \
                 (optionally wrapped in `Option<...>` and/or `Lazy<...>`) \
                 — got: {}",
                quote!(#ty),
            ),
        ));
    };

    Ok((optional, lazy, inner))
}

/// Match `Wrapper<Inner>` by path-tail (last segment ident == `wrapper_name`).
/// Returns `Inner` if matched.
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

/// Generate the `Dependencies` registration calls for slot fields.
///
/// Emits one `.slot_field(SlotField { ... })` call per parsed slot.
pub(crate) fn emit_slot_field_registrations(slots: &[ParsedSlotField]) -> TokenStream2 {
    let calls: Vec<TokenStream2> = slots
        .iter()
        .map(|slot| {
            let slot_key = slot.slot_key();
            let inner_ty = &slot.inner_type;
            let required = !slot.optional;
            let lazy = slot.lazy;
            let kind_tokens = match slot.kind {
                SlotKind::Resource => quote! {
                    ::nebula_core::SlotKind::Resource {
                        type_id: ::std::any::TypeId::of::<#inner_ty>(),
                        type_name: ::std::any::type_name::<#inner_ty>(),
                        key: <#inner_ty as ::nebula_resource::Resource>::key(),
                    }
                },
                SlotKind::Credential => quote! {
                    ::nebula_core::SlotKind::Credential {
                        type_id: ::std::any::TypeId::of::<#inner_ty>(),
                        type_name: ::std::any::type_name::<#inner_ty>(),
                        key: ::nebula_core::CredentialKey::new(
                            <#inner_ty as ::nebula_credential::Credential>::KEY
                        ).expect("credential KEY must be a valid CredentialKey"),
                    }
                },
            };
            quote! {
                .slot_field(::nebula_core::SlotField {
                    slot_key: #slot_key,
                    default_id: #slot_key,
                    kind: #kind_tokens,
                    required: #required,
                    lazy: #lazy,
                })
            }
        })
        .collect();

    quote! { #(#calls)* }
}

/// Generate the field-resolution body for `FromWorkflowNode::from_workflow_node`.
///
/// Each emitted statement reads the slot binding from the node, falling
/// back to the slot's `default_id`, then calls into `ActionContextExt`
/// (or `Lazy::with_value` etc.) and binds the result to a local matching
/// the field name.
pub(crate) fn emit_slot_resolution_block(slots: &[ParsedSlotField]) -> (TokenStream2, Vec<Ident>) {
    let mut stmts = Vec::with_capacity(slots.len());
    let mut idents = Vec::with_capacity(slots.len());

    for slot in slots {
        let field = &slot.field_ident;
        let slot_key = slot.slot_key();
        let slot_key_lit = slot_key.as_str();
        let inner_ty = &slot.inner_type;

        let binding_call = match slot.kind {
            SlotKind::Resource => quote! { node.resource_binding(#slot_key_lit) },
            SlotKind::Credential => quote! { node.credential_binding(#slot_key_lit) },
        };

        // Resolution call dispatched through `ActionContextExt`.
        let resolve_call = match slot.kind {
            SlotKind::Resource => quote! {
                <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                    ::acquire_resource_by_id::<#inner_ty>(ctx, slot_id)
                    .await
            },
            SlotKind::Credential => quote! {
                <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                    ::resolve_credential_by_id::<#inner_ty>(ctx, slot_id)
                    .await
            },
        };

        let kind_word = slot.kind_word();
        let optional = slot.optional;
        let lazy = slot.lazy;

        // Build the per-slot resolution block. Each shape produces a
        // value of the field's declared type.
        let stmt = match (optional, lazy) {
            (false, false) => quote! {
                let #field = {
                    let slot_id = #binding_call.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => guard,
                        Err(e) => {
                            return Err(::nebula_action::ActionError::fatal(
                                format!(
                                    "failed to resolve {} slot `{}` (id `{}`): {}",
                                    #kind_word, #slot_key_lit, slot_id, e,
                                )
                            ));
                        }
                    }
                };
            },
            (true, false) => quote! {
                let #field = {
                    let slot_id = #binding_call.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => Some(guard),
                        Err(_) => None,
                    }
                };
            },
            (false, true) => quote! {
                let #field = {
                    let slot_id = #binding_call.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => ::nebula_core::sync::Lazy::with_value(guard),
                        Err(e) => {
                            return Err(::nebula_action::ActionError::fatal(
                                format!(
                                    "failed to resolve {} slot `{}` (id `{}`): {}",
                                    #kind_word, #slot_key_lit, slot_id, e,
                                )
                            ));
                        }
                    }
                };
            },
            (true, true) => quote! {
                let #field = {
                    let slot_id = #binding_call.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => Some(::nebula_core::sync::Lazy::with_value(guard)),
                        Err(_) => None,
                    }
                };
            },
        };
        stmts.push(stmt);
        idents.push(field.clone());
    }

    let block = quote! { #(#stmts)* };
    (block, idents)
}
