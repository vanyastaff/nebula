//! Field-level slot detection for `#[derive(Action)]` Variant A.
//!
//! Walks the struct fields and identifies `#[resource(...)]` or
//! `#[credential(...)]` attributes.
//!
//! A `#[resource]` field holds the row's per-unit checkout facade —
//! `ResourceHandle<R>` (required) or `Option<ResourceHandle<R>>` (optional).
//! Resolving it checks nothing out, so `Lazy<ResourceHandle<R>>` is rejected.
//! A `ResourceGuard<R>` lease in any wrapper is refused with a targeted
//! diagnostic: since 0.27.0 the handle is the only resource capability an
//! action can name, because a lease bypasses the effect journal.
//!
//! A `#[credential]` field type must follow one of:
//!
//! - `CredentialGuard<C>` — required + eager
//! - `Option<CredentialGuard<C>>` — optional + eager
//!
//! Lazy credential wrappers are rejected: the old expansion resolved the guard
//! eagerly before wrapping it, so it never deferred acquisition.
//!
//! Detection is by path-tail name (last `PathSegment::ident`) so the
//! macro accepts both bare `ResourceHandle<...>` and fully-qualified
//! `nebula_resource::call::ResourceHandle<...>`.

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

    let FieldShape { optional, inner } = decode_field_type(&field.ty, kind)?;

    Ok(ParsedSlotField {
        field_ident,
        key_override,
        kind,
        optional,
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
    /// The concrete `R` or `C` underneath the wrappers.
    inner: Type,
}

/// The diagnostic for a `ResourceGuard<T>` resource slot in any wrapper.
const REMOVED_RESOURCE_GUARD_SLOT: &str = "`ResourceGuard<T>` slots were removed in 0.27.0; \
     hold `ResourceHandle<T>` — a lease bypasses the effect journal";

/// The old wrapper never deferred credential resolution.
const REMOVED_LAZY_CREDENTIAL_SLOT: &str = "lazy credential slots are unsupported: the old wrapper resolved eagerly; use `CredentialGuard<T>` or `Option<CredentialGuard<T>>` instead";

/// Decode the field type, recognising the allowed shapes.
fn decode_field_type(ty: &Type, kind: SlotKind) -> Result<FieldShape> {
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

    // The facade's pre-0.22.0 name gets a hint instead of the generic
    // wrong-type error.
    if kind == SlotKind::Resource && strip_path_tail(&after_lazy, "ManagedRow").is_some() {
        return Err(syn::Error::new_spanned(
            ty,
            "`ManagedRow` was renamed to `ResourceHandle`; write `ResourceHandle<T>`",
        ));
    }

    match kind {
        SlotKind::Resource => {
            // A lease in any wrapper gets the migration hint rather than the
            // generic wrong-type error.
            if strip_path_tail(&after_lazy, "ResourceGuard").is_some() {
                return Err(syn::Error::new_spanned(ty, REMOVED_RESOURCE_GUARD_SLOT));
            }
            let Some(inner) = strip_path_tail(&after_lazy, "ResourceHandle") else {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "field with `#[resource]` must have type `ResourceHandle<T>` \
                         (optionally wrapped in `Option<...>`) — got: {}",
                        quote!(#ty),
                    ),
                ));
            };
            if lazy {
                return Err(syn::Error::new_spanned(
                    ty,
                    "a ResourceHandle acquires nothing at resolution; drop `Lazy`",
                ));
            }
            Ok(FieldShape { optional, inner })
        },
        SlotKind::Credential => {
            if lazy {
                return Err(syn::Error::new_spanned(ty, REMOVED_LAZY_CREDENTIAL_SLOT));
            }
            let Some(inner) = strip_path_tail(&after_lazy, "CredentialGuard") else {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "field with `#[credential]` must have type `CredentialGuard<T>` \
                         (optionally wrapped in `Option<...>`) — got: {}",
                        quote!(#ty),
                    ),
                ));
            };
            Ok(FieldShape { optional, inner })
        },
    }
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
                    lazy: false,
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
/// diagnostics, then calls into `ActionContextExt` and binds the result to a
/// local matching the field name. Credential
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
        let kind_word = slot.kind_word();
        let optional = slot.optional;

        // A resource slot is always a `ResourceHandle<R>`: it checks nothing
        // out, so its resolution is synchronous. Resource handles already
        // return the action-layer error classification chosen by the
        // accessor. Preserve it verbatim: wrapping a revoked or suspended
        // row in `fatal` would disable the engine retry policy. An optional
        // row uses the typed try seam so only genuine absence is `None`;
        // lifecycle and type failures still propagate. A durable execution
        // graph carries no concrete selectors on its projected node, so the
        // no-binding path addresses the activated row by the provider
        // contract key rather than by the authored field name.
        if slot.kind == SlotKind::Resource {
            let stmt = if optional {
                quote! {
                    let #field = {
                        if let Some(slot_id) = #binding_call {
                            match <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::resource_handle_by_id::<#inner_ty>(ctx, slot_id)
                            {
                                Ok(row) => Some(row),
                                Err(error) => return Err(error),
                            }
                        } else {
                            let resource_key = <#inner_ty as ::nebula_resource::resource::Provider>::key();
                            match <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::try_resource_handle_by_id::<#inner_ty>(ctx, resource_key.as_str())
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
                                ::resource_handle_by_id::<#inner_ty>(ctx, slot_id)
                        } else {
                            let resource_key = <#inner_ty as ::nebula_resource::resource::Provider>::key();
                            <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                                ::resource_handle_by_id::<#inner_ty>(ctx, resource_key.as_str())
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

        // Credential resolution dispatched through `ActionContextExt`, by
        // the declared slot key: start admission already resolved any
        // authored selector into the durable binding manifest.
        let resolve_call = quote! {
            <dyn ::nebula_action::ActionContext as ::nebula_action::ActionContextExt>
                ::resolve_credential_by_id::<#inner_ty>(ctx, #slot_key_lit)
                .await
        };

        // Build the per-slot resolution block. Each shape produces a
        // value of the field's declared type.
        //
        // Optional-slot discipline: `None` means "binding absent" — no
        // explicit binding AND the default-id resolution also returned
        // nothing. A binding that is present but fails resolution (e.g.
        // the credential id is invalid or inaccessible) is a
        // hard error and must propagate, not silently become `None`.
        // The `binding_present` variable captures whether an explicit
        // binding was configured so the error arm can distinguish the
        // two cases.
        let stmt = if optional {
            quote! {
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
            }
        } else {
            quote! {
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
            }
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
            inner_type: syn::parse_quote!(DemoCredential),
        }
    }

    fn decoded(ty: Type) -> Result<FieldShape> {
        decode_field_type(&ty, SlotKind::Resource)
    }

    #[test]
    fn a_resource_handle_field_decodes_required_or_optional() {
        let shape = decoded(syn::parse_quote!(ResourceHandle<Db>)).expect("required row");
        assert!(!shape.optional);
        let inner = &shape.inner;
        assert_eq!(quote!(#inner).to_string(), "Db");

        let shape = decoded(syn::parse_quote!(
            Option<nebula_sdk::integration::resource::ResourceHandle<Db>>
        ))
        .expect("optional row");
        assert!(shape.optional);
    }

    #[test]
    fn a_resource_guard_in_any_wrapper_points_at_the_handle() {
        for ty in [
            syn::parse_quote!(ResourceGuard<Db>),
            syn::parse_quote!(nebula_resource::ResourceGuard<Db>),
            syn::parse_quote!(Option<ResourceGuard<Db>>),
            syn::parse_quote!(Lazy<ResourceGuard<Db>>),
            syn::parse_quote!(Option<Lazy<ResourceGuard<Db>>>),
        ] {
            let Err(error) = decoded(ty) else {
                panic!("a lease slot must be refused");
            };
            assert_eq!(error.to_string(), REMOVED_RESOURCE_GUARD_SLOT);
        }
    }

    #[test]
    fn a_lazy_resource_handle_is_rejected() {
        for ty in [
            syn::parse_quote!(Lazy<ResourceHandle<Db>>),
            syn::parse_quote!(Option<Lazy<ResourceHandle<Db>>>),
        ] {
            let Err(error) = decoded(ty) else {
                panic!("a lazy row must be rejected");
            };
            assert!(error.to_string().contains("drop `Lazy`"), "{error}");
        }
    }

    #[test]
    fn the_old_row_facade_name_points_at_its_new_name() {
        for ty in [
            syn::parse_quote!(ManagedRow<Db>),
            syn::parse_quote!(Option<nebula_sdk::integration::resource::ManagedRow<Db>>),
            syn::parse_quote!(Lazy<ManagedRow<Db>>),
        ] {
            let Err(error) = decoded(ty) else {
                panic!("the old name must be refused");
            };
            assert!(
                error.to_string().contains("renamed to `ResourceHandle`"),
                "{error}"
            );
        }
    }

    #[test]
    fn credential_guards_decode_required_or_optional() {
        for (ty, optional) in [
            (syn::parse_quote!(CredentialGuard<Auth>), false),
            (
                syn::parse_quote!(Option<nebula_credential::CredentialGuard<Auth>>),
                true,
            ),
        ] {
            let shape = decode_field_type(&ty, SlotKind::Credential).expect("supported guard");
            assert_eq!(shape.optional, optional);
            let inner = &shape.inner;
            assert_eq!(quote!(#inner).to_string(), "Auth");
        }
    }

    #[test]
    fn lazy_credential_guards_are_rejected() {
        for ty in [
            syn::parse_quote!(Lazy<CredentialGuard<Auth>>),
            syn::parse_quote!(Option<Lazy<CredentialGuard<Auth>>>),
            syn::parse_quote!(nebula_core::sync::Lazy<nebula_credential::CredentialGuard<Auth>>),
            syn::parse_quote!(
                Option<nebula_core::sync::Lazy<nebula_credential::CredentialGuard<Auth>>>
            ),
        ] {
            let error = decode_field_type(&ty, SlotKind::Credential)
                .err()
                .expect("lazy guards must be rejected");
            assert_eq!(error.to_string(), REMOVED_LAZY_CREDENTIAL_SLOT);
        }
    }

    #[test]
    fn a_credential_field_never_decodes_as_a_row() {
        let error = decode_field_type(&syn::parse_quote!(ResourceHandle<Db>), SlotKind::Credential)
            .err()
            .expect("credentials have no row facade");
        assert!(error.to_string().contains("CredentialGuard<T>"), "{error}");
    }

    #[test]
    fn a_row_field_resolves_synchronously_through_resource_handle_by_id() {
        for optional in [false, true] {
            let row = ParsedSlotField {
                field_ident: format_ident!("db"),
                optional,
                inner_type: syn::parse_quote!(Db),
                ..slot(SlotKind::Resource)
            };
            let (block, idents) = emit_slot_resolution_block(&[row]);
            let expanded = block.to_string();
            assert!(
                expanded.contains("resource_handle_by_id :: < Db > (ctx , slot_id)"),
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
                expanded.contains("try_resource_handle_by_id :: < Db >"),
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

        assert!(expanded.contains("if let Some (slot_id) = node . resource_binding"));
        assert!(expanded.contains("resource_handle_by_id :: < DemoCredential > (ctx , slot_id)"));
    }
}
