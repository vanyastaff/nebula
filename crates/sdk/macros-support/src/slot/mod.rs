//! Pure parsing for dependency declarations, separate from value schemas.
//!
//! Parsing establishes syntax and receiver shape only. The owning leaf must
//! admit scheme compatibility, binding authority and checked conditions against
//! the exact associated input/config schema before using a declaration.

use std::collections::BTreeSet;

use syn::{
    Attribute, Expr, Field, Fields, GenericArgument, Ident, LitStr, PathArguments, Result, Token,
    Type, parenthesized,
    parse::{Parse, ParseStream},
};

/// Integration receiver on which slot declarations are being expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    /// An action receiver; optional guards and handles can represent inactivity.
    Action,
    /// A resource receiver; only credential cells are accepted.
    Resource,
    /// Credential setup data is value-only and cannot declare dependency slots.
    CredentialProperties,
}

/// Dependency kind, independent of catalog or concrete instance identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A projected auth scheme, never an inferred credential provider.
    Credential,
    /// A provider's managed per-unit resource facade.
    Resource,
}

/// Whether an active declaration requires a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingPolicy {
    /// Active absence is an error.
    Required,
    /// Only authorized unconfigured/default absence is permitted.
    Optional,
}

/// A syntax-checked slot declaration. It conveys no runtime authority.
#[derive(Debug, Clone)]
pub struct SlotField {
    /// Rust receiver field, unaffected by serde renames.
    pub field_ident: Ident,
    /// Local declaration/binding address, unique across kinds on the receiver.
    pub key: String,
    /// Optional catalog description.
    pub purpose: Option<LitStr>,
    /// Dependency category.
    pub kind: SlotKind,
    /// Requiredness of an active binding, independent of the Rust wrapper.
    pub binding: BindingPolicy,
    /// Whether the action wrapper can represent absence.
    pub optional: bool,
    /// Scheme type for credential slots, provider type for resource slots.
    pub inner_type: Type,
    /// Original checked DSL syntax; the leaf lowers and admits it against its DTO.
    pub bind_when: Option<Expr>,
}

struct Arguments {
    kind: SlotKind,
    key: Option<LitStr>,
    purpose: Option<LitStr>,
    binding: Option<BindingPolicy>,
    bind_when: Option<Expr>,
}

impl Parse for Arguments {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let kind_name: Ident = input.parse()?;
        let kind = match kind_name.to_string().as_str() {
            "credential" => SlotKind::Credential,
            "resource" => SlotKind::Resource,
            _ => {
                return Err(syn::Error::new(
                    kind_name.span(),
                    "slot requires exactly one kind: credential or resource",
                ));
            },
        };
        let mut args = Self {
            kind,
            key: None,
            purpose: None,
            binding: None,
            bind_when: None,
        };
        let mut seen = BTreeSet::new();
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let name: Ident = input.parse()?;
            let word = name.to_string();
            if !seen.insert(word.clone()) {
                return Err(syn::Error::new(name.span(), "duplicate slot argument"));
            }
            match word.as_str() {
                "key" | "purpose" => {
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    if word == "key" {
                        args.key = Some(value);
                    } else {
                        args.purpose = Some(value);
                    }
                },
                "binding" => {
                    input.parse::<Token![=]>()?;
                    let value: Ident = input.parse()?;
                    args.binding = Some(match value.to_string().as_str() {
                        "required" => BindingPolicy::Required,
                        "optional" => BindingPolicy::Optional,
                        _ => {
                            return Err(syn::Error::new(
                                value.span(),
                                "binding must be required or optional",
                            ));
                        },
                    });
                },
                "bind_when" => {
                    let content;
                    parenthesized!(content in input);
                    let condition: Expr = content.parse()?;
                    if !content.is_empty() {
                        return Err(content.error("bind_when accepts exactly one condition"));
                    }
                    validate_condition(&condition, ConditionScope::AssociatedData)?;
                    args.bind_when = Some(condition);
                },
                _ => {
                    return Err(syn::Error::new(
                        name.span(),
                        "unknown slot argument; expected key, purpose, binding or bind_when",
                    ));
                },
            }
        }
        Ok(args)
    }
}

/// Parse all new slot declarations and reject duplicate keys across kinds.
///
/// Legacy attributes on other fields are left to their existing parsers. A
/// field cannot mix legacy slot/value helpers with the new declaration.
pub fn parse_slots(fields: &Fields, receiver: ReceiverKind) -> Result<Vec<SlotField>> {
    let mut slots = Vec::new();
    let mut keys = BTreeSet::new();
    for field in fields {
        if let Some(slot) = parse_slot(field, receiver)? {
            if !keys.insert(slot.key.clone()) {
                return Err(syn::Error::new_spanned(
                    &slot.field_ident,
                    "duplicate slot key across dependency declarations",
                ));
            }
            slots.push(slot);
        }
    }
    Ok(slots)
}

/// Parse a single field; return `None` when it has no new slot attribute.
pub fn parse_slot(field: &Field, receiver: ReceiverKind) -> Result<Option<SlotField>> {
    let mut attributes = field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("slot"));
    let Some(attribute) = attributes.next() else {
        return Ok(None);
    };
    if let Some(duplicate) = attributes.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "only one slot declaration is allowed per field",
        ));
    }
    if receiver == ReceiverKind::CredentialProperties {
        return Err(syn::Error::new_spanned(
            attribute,
            "credential properties are value-only; slots belong on action or resource receivers",
        ));
    }
    if let Some(conflict) = field.attrs.iter().find(|attr| conflicts_with_slot(attr)) {
        return Err(syn::Error::new_spanned(
            conflict,
            "slot declarations cannot mix with value or legacy dependency attributes",
        ));
    }
    let field_ident = field.ident.clone().ok_or_else(|| {
        syn::Error::new_spanned(field, "slot declarations require named receiver fields")
    })?;
    let args: Arguments = attribute.parse_args()?;
    let (optional, inner_type) = decode_shape(&field.ty, receiver, args.kind)?;
    let binding = args.binding.unwrap_or(if optional {
        BindingPolicy::Optional
    } else {
        BindingPolicy::Required
    });
    if receiver == ReceiverKind::Action
        && !optional
        && (binding == BindingPolicy::Optional || args.bind_when.is_some())
    {
        return Err(syn::Error::new_spanned(
            attribute,
            "optional binding or bind_when requires an Option action slot",
        ));
    }
    let key = match args.key {
        Some(key) => {
            validate_key(&key)?;
            key.value()
        },
        None => field_ident.to_string().trim_start_matches("r#").to_owned(),
    };
    Ok(Some(SlotField {
        field_ident,
        key,
        purpose: args.purpose,
        kind: args.kind,
        binding,
        optional,
        inner_type,
        bind_when: args.bind_when,
    }))
}

fn conflicts_with_slot(attribute: &Attribute) -> bool {
    ["property", "field", "validate", "credential", "resource"]
        .iter()
        .any(|name| attribute.path().is_ident(name))
}

fn generic_type<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let last = path.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn decode_shape(ty: &Type, receiver: ReceiverKind, kind: SlotKind) -> Result<(bool, Type)> {
    let invalid = || {
        syn::Error::new_spanned(
            ty,
            "unsupported slot receiver type; actions use CredentialGuard<S> or ResourceHandle<R> (optionally Option), resources use CredentialSlot<S> or SlotCell<CredentialGuard<S>>",
        )
    };
    match receiver {
        ReceiverKind::Action => {
            action_migration_diagnostic(ty, kind)?;
            let after_option = generic_type(ty, "Option");
            let wrapper = match kind {
                SlotKind::Credential => "CredentialGuard",
                SlotKind::Resource => "ResourceHandle",
            };
            let inner = generic_type(after_option.unwrap_or(ty), wrapper).ok_or_else(invalid)?;
            Ok((after_option.is_some(), inner.clone()))
        },
        ReceiverKind::Resource if kind == SlotKind::Credential => {
            if let Some(inner) = generic_type(ty, "CredentialSlot") {
                return Ok((false, inner.clone()));
            }
            let cell = generic_type(ty, "SlotCell").ok_or_else(invalid)?;
            let scheme = generic_type(cell, "CredentialGuard").ok_or_else(invalid)?;
            Ok((false, scheme.clone()))
        },
        ReceiverKind::Resource | ReceiverKind::CredentialProperties => Err(invalid()),
    }
}

fn validate_key(key: &LitStr) -> Result<()> {
    if key.value().is_empty() {
        Err(syn::Error::new_spanned(key, "slot key cannot be empty"))
    } else {
        Ok(())
    }
}

fn action_migration_diagnostic(ty: &Type, kind: SlotKind) -> Result<()> {
    let mut inner = ty;
    let mut lazy = false;
    loop {
        if let Some(next) = generic_type(inner, "Option") {
            inner = next;
        } else if let Some(next) = generic_type(inner, "Lazy") {
            lazy = true;
            inner = next;
        } else {
            break;
        }
    }
    let message = match kind {
        SlotKind::Credential if lazy && generic_type(inner, "CredentialGuard").is_some() => Some(
            "lazy credential slots are unsupported: the old wrapper resolved eagerly; use `CredentialGuard<T>` or `Option<CredentialGuard<T>>` instead",
        ),
        SlotKind::Resource if generic_type(inner, "ResourceGuard").is_some() => Some(
            "`ResourceGuard<T>` slots were removed in 0.27.0; hold `ResourceHandle<T>` — a lease bypasses the effect journal",
        ),
        SlotKind::Resource if generic_type(inner, "ManagedRow").is_some() => {
            Some("`ManagedRow` was renamed to `ResourceHandle`; write `ResourceHandle<T>`")
        },
        SlotKind::Resource if lazy && generic_type(inner, "ResourceHandle").is_some() => {
            Some("a ResourceHandle acquires nothing at resolution; drop `Lazy`")
        },
        _ => None,
    };
    match message {
        Some(message) => Err(syn::Error::new_spanned(ty, message)),
        None => Ok(()),
    }
}

mod slot_condition;

pub use slot_condition::{ConditionScope, validate_condition};

#[cfg(test)]
mod tests;
