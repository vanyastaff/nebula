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
//! A `#[resource]` field may instead hold the row's per-unit checkout
//! facade: `ManagedRow<R>` (required) or `Option<ManagedRow<R>>` (optional).
//! Resolving it checks nothing out, so `Lazy<ManagedRow<R>>` is rejected.
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
    /// Whether a resource field holds the row facade `ManagedRow<R>`
    /// rather than a `ResourceGuard<R>` lease.
    pub row: bool,
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

    let FieldShape {
        optional,
        lazy,
        row,
        inner,
    } = decode_field_type(&field.ty, kind)?;

    Ok(ParsedSlotField {
        field_ident,
        key_override,
        kind,
        optional,
        lazy,
        row,
        inner_type: inner,
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

/// A decoded slot field type.
struct FieldShape {
    /// Wrapped in `Option<...>`.
    optional: bool,
    /// Wrapped in `Lazy<...>`.
    lazy: bool,
    /// A `ManagedRow<R>` resource field.
    row: bool,
    /// The concrete `R` or `C` underneath the wrappers.
    inner: Type,
}

/// Decode the field type, recognising the allowed shapes.
fn decode_field_type(ty: &Type, kind: SlotKind) -> Result<FieldShape> {
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

    // A resource field may hold the row facade instead of a lease.
    if kind == SlotKind::Resource
        && let Some(inner) = strip_path_tail(&after_lazy, "ManagedRow")
    {
        if lazy {
            return Err(syn::Error::new_spanned(
                ty,
                "a ManagedRow acquires nothing at resolution; drop `Lazy`",
            ));
        }
        return Ok(FieldShape {
            optional,
            lazy,
            row: true,
            inner,
        });
    }

    // The remaining type must be ResourceGuard<R> / CredentialGuard<C>.
    let Some(inner) = strip_path_tail(&after_lazy, guard_ident) else {
        let kw = match kind {
            SlotKind::Resource => "resource",
            SlotKind::Credential => "credential",
        };
        let row_shape = match kind {
            SlotKind::Resource => ", or `ManagedRow<T>` (optionally wrapped in `Option<...>`)",
            SlotKind::Credential => "",
        };
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "field with `#[{kw}]` must have type `{guard_ident}<T>` \
                 (optionally wrapped in `Option<...>` and/or `Lazy<...>`){row_shape} \
                 — got: {}",
                quote!(#ty),
            ),
        ));
    };

    Ok(FieldShape {
        optional,
        lazy,
        row: false,
        inner,
    })
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
                        key: <#inner_ty as ::nebula_resource::resource::Provider>::key(),
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
                    purpose: ::core::option::Option::None,
                })
            }
        })
        .collect();

    quote! { #(#calls)* }
}

/// Generate the field-resolution body for `FromWorkflowNode::from_workflow_node`.
///
/// Each emitted statement reads the authored slot binding from the node for
/// diagnostics, then calls into `ActionContextExt` (or `Lazy::with_value`
/// etc.) and binds the result to a local matching the field name. Credential
/// slots resolve through the declared slot key because start admission already
/// resolved any authored selector into the durable binding manifest.
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
        let lookup_id = match slot.kind {
            SlotKind::Resource => quote! { slot_id },
            SlotKind::Credential => quote! { #slot_key_lit },
        };

        // Resolution call dispatched through `ActionContextExt`. A row
        // facade checks nothing out, so its resolution is synchronous.
        let resolve_call = match slot.kind {
            SlotKind::Resource if slot.row => quote! {
                <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                    ::managed_row_by_id::<#inner_ty>(ctx, #lookup_id)
            },
            SlotKind::Resource => quote! {
                <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                    ::acquire_resource_by_id::<#inner_ty>(ctx, #lookup_id)
                    .await
            },
            SlotKind::Credential => quote! {
                <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                    ::resolve_credential_by_id::<#inner_ty>(ctx, #lookup_id)
                    .await
            },
        };

        let kind_word = slot.kind_word();
        let optional = slot.optional;
        let lazy = slot.lazy;

        // Managed rows already return the action-layer error classification
        // chosen by the accessor. Preserve it verbatim: wrapping a revoked or
        // suspended row in `fatal` would disable the engine retry policy. An
        // optional row uses the typed try seam so only genuine absence is
        // `None`; lifecycle and type failures still propagate. A durable
        // execution graph carries no concrete selectors on its projected
        // node, so the no-binding path addresses the activated row by the
        // provider contract key rather than by the authored field name.
        if slot.row {
            debug_assert!(!lazy, "managed rows cannot be lazy");
            let stmt = if optional {
                quote! {
                    let #field = {
                        if let Some(slot_id) = #binding_call {
                            match <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::managed_row_by_id::<#inner_ty>(ctx, slot_id)
                            {
                                Ok(row) => Some(row),
                                Err(error) => return Err(error),
                            }
                        } else {
                            let resource_key = <#inner_ty as ::nebula_resource::resource::Provider>::key();
                            match <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::try_managed_row_by_id::<#inner_ty>(ctx, resource_key.as_str())
                            {
                                Ok(row) => row,
                                Err(error) => return Err(error),
                            }
                        }
                    };
                }
            } else {
                quote! {
                    let #field = {
                        let resolved = if let Some(slot_id) = #binding_call {
                            <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::managed_row_by_id::<#inner_ty>(ctx, slot_id)
                        } else {
                            let resource_key = <#inner_ty as ::nebula_resource::resource::Provider>::key();
                            <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::managed_row_by_id::<#inner_ty>(ctx, resource_key.as_str())
                        };
                        match resolved {
                            Ok(row) => row,
                            Err(error) => return Err(error),
                        }
                    };
                }
            };
            stmts.push(stmt);
            idents.push(field.clone());
            continue;
        }

        // Build the per-slot resolution block. Each shape produces a
        // value of the field's declared type.
        //
        // Optional-slot discipline: `None` means "binding absent" — no
        // explicit binding AND the default-id resolution also returned
        // nothing. A binding that is present but fails resolution (e.g.
        // the resource/credential id is invalid or inaccessible) is a
        // hard error and must propagate, not silently become `None`.
        // The `binding_present` variable captures whether an explicit
        // binding was configured so the error arm can distinguish the
        // two cases.
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
                    let explicit_binding = #binding_call;
                    let slot_id = explicit_binding.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => Some(guard),
                        Err(e) => {
                            if explicit_binding.is_some() {
                                // Binding was configured but resolution failed —
                                // propagate as a hard error, not a silent None.
                                return Err(::nebula_action::ActionError::fatal(
                                    format!(
                                        "failed to resolve explicitly-bound {} slot `{}` (id `{}`): {}",
                                        #kind_word, #slot_key_lit, slot_id, e,
                                    )
                                ));
                            }
                            // No explicit binding; default-id resolution returned
                            // nothing — treat as absent.
                            let _ = e;
                            None
                        }
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
                    let explicit_binding = #binding_call;
                    let slot_id = explicit_binding.unwrap_or(#slot_key_lit);
                    match #resolve_call {
                        Ok(guard) => Some(::nebula_core::sync::Lazy::with_value(guard)),
                        Err(e) => {
                            if explicit_binding.is_some() {
                                return Err(::nebula_action::ActionError::fatal(
                                    format!(
                                        "failed to resolve explicitly-bound {} slot `{}` (id `{}`): {}",
                                        #kind_word, #slot_key_lit, slot_id, e,
                                    )
                                ));
                            }
                            let _ = e;
                            None
                        }
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

#[cfg(test)]
mod tests {
    use quote::format_ident;

    use super::*;

    fn slot(kind: SlotKind) -> ParsedSlotField {
        ParsedSlotField {
            field_ident: format_ident!("auth"),
            key_override: None,
            kind,
            optional: false,
            lazy: false,
            row: false,
            inner_type: syn::parse_quote!(DemoCredential),
        }
    }

    fn decoded(ty: Type) -> Result<FieldShape> {
        decode_field_type(&ty, SlotKind::Resource)
    }

    #[test]
    fn a_managed_row_field_decodes_required_or_optional() {
        let shape = decoded(syn::parse_quote!(ManagedRow<Db>)).expect("required row");
        assert!(shape.row && !shape.optional && !shape.lazy);
        let inner = &shape.inner;
        assert_eq!(quote!(#inner).to_string(), "Db");

        let shape = decoded(syn::parse_quote!(
            Option<nebula_sdk::integration::resource::ManagedRow<Db>>
        ))
        .expect("optional row");
        assert!(shape.row && shape.optional && !shape.lazy);

        let shape = decoded(syn::parse_quote!(ResourceGuard<Db>)).expect("a lease");
        assert!(!shape.row);
    }

    #[test]
    fn a_lazy_managed_row_is_rejected() {
        for ty in [
            syn::parse_quote!(Lazy<ManagedRow<Db>>),
            syn::parse_quote!(Option<Lazy<ManagedRow<Db>>>),
        ] {
            let Err(error) = decoded(ty) else {
                panic!("a lazy row must be rejected");
            };
            assert!(error.to_string().contains("drop `Lazy`"), "{error}");
        }
    }

    #[test]
    fn a_credential_field_never_decodes_as_a_row() {
        let error = decode_field_type(&syn::parse_quote!(ManagedRow<Db>), SlotKind::Credential)
            .err()
            .expect("credentials have no row facade");
        assert!(error.to_string().contains("CredentialGuard<T>"), "{error}");
    }

    #[test]
    fn a_row_field_resolves_synchronously_through_managed_row_by_id() {
        for optional in [false, true] {
            let row = ParsedSlotField {
                field_ident: format_ident!("db"),
                row: true,
                optional,
                inner_type: syn::parse_quote!(Db),
                ..slot(SlotKind::Resource)
            };
            let (block, idents) = emit_slot_resolution_block(&[row]);
            let expanded = block.to_string();
            assert!(
                expanded.contains("managed_row_by_id :: < Db > (ctx , slot_id)"),
                "{expanded}"
            );
            assert!(
                expanded.contains("< Db as :: nebula_resource :: resource :: Provider > :: key ()"),
                "{expanded}"
            );
            assert!(expanded.contains("resource_key . as_str ()"), "{expanded}");
            assert!(!expanded.contains("acquire_resource_by_id"), "{expanded}");
            assert!(!expanded.contains(". await"), "{expanded}");
            assert_eq!(
                expanded.contains("try_managed_row_by_id :: < Db >"),
                optional,
                "{expanded}"
            );
            assert_eq!(expanded.contains("Some (row)"), optional, "{expanded}");
            assert_eq!(idents, [format_ident!("db")]);
        }
    }

    #[test]
    fn a_row_field_registers_as_its_resource() {
        let row = ParsedSlotField {
            row: true,
            inner_type: syn::parse_quote!(Db),
            ..slot(SlotKind::Resource)
        };
        let registration = emit_slot_field_registrations(&[row]).to_string();
        assert!(
            registration.contains("< Db as :: nebula_resource :: resource :: Provider > :: key ()"),
            "{registration}"
        );
        assert!(
            registration.contains("TypeId :: of :: < Db >"),
            "{registration}"
        );
    }

    #[test]
    fn credential_resolution_uses_slot_key_not_authored_selector() {
        let (block, _) = emit_slot_resolution_block(&[slot(SlotKind::Credential)]);
        let expanded = block.to_string();

        assert!(expanded.contains("let slot_id = node . credential_binding"));
        assert!(
            expanded.contains("resolve_credential_by_id :: < DemoCredential > (ctx , \"auth\")")
        );
        assert!(
            !expanded.contains("resolve_credential_by_id :: < DemoCredential > (ctx , slot_id)")
        );
    }

    #[test]
    fn resource_resolution_still_uses_selected_resource_id() {
        let (block, _) = emit_slot_resolution_block(&[slot(SlotKind::Resource)]);
        let expanded = block.to_string();

        assert!(expanded.contains("let slot_id = node . resource_binding"));
        assert!(expanded.contains("acquire_resource_by_id :: < DemoCredential > (ctx , slot_id)"));
    }
}
