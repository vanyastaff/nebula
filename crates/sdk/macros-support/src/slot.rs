//! `#[slot]` field declarations, shared by the component derives.
//!
//! A slot is declared once, on the field that holds its value:
//!
//! ```text
//! #[slot] db: ResourceHandle<Postgres>,
//! #[slot(key = "audit", purpose = "Audit log")] log: Option<ResourceHandle<Log>>,
//! ```
//!
//! The parser never looks at the field type. What the slot points at and
//! whether it is required follow from the type through the `SlotValue` trait,
//! so the derive writes `Slot::of::<FieldType>(key)` and the compiler checks
//! the rest. Aliases and full paths therefore work, and a field of the wrong
//! type fails with that trait's message.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Field, Fields, Ident, LitStr, Result, Token, Type, parse::ParseStream};

/// One `#[slot]` field.
#[derive(Debug, Clone)]
pub struct SlotField {
    /// The struct field, which also names the slot by default.
    pub field_ident: Ident,
    /// The field type, which holds the slot's value.
    pub ty: Type,
    /// The slot key: `key = "..."` or the field name.
    pub key: String,
    /// What the slot is for, shown in the catalog.
    pub purpose: Option<LitStr>,
}

/// Field attributes that declared slots before `#[slot]`.
const REPLACED_ATTRIBUTES: [&str; 2] = ["resource", "credential"];

/// Parses every `#[slot]` field of a derive input's fields.
///
/// Fields without `#[slot]` are left alone. A tuple struct may not declare
/// slots, since a slot is addressed by its field name.
///
/// # Errors
///
/// Returns a spanned error for a malformed attribute, an invalid or repeated
/// key, a slot on a tuple field, or a field still using `#[resource]` or
/// `#[credential]`.
pub fn parse_slots(fields: &Fields) -> Result<Vec<SlotField>> {
    let mut slots = Vec::new();
    let mut keys = BTreeSet::new();
    for field in fields {
        reject_replaced_attributes(field)?;
        let Some(slot) = parse_slot(field)? else {
            continue;
        };
        if !keys.insert(slot.key.clone()) {
            return Err(syn::Error::new_spanned(
                &slot.field_ident,
                format!("slot key `{}` is declared twice", slot.key),
            ));
        }
        slots.push(slot);
    }
    Ok(slots)
}

fn reject_replaced_attributes(field: &Field) -> Result<()> {
    for attribute in &field.attrs {
        if let Some(name) = REPLACED_ATTRIBUTES
            .iter()
            .find(|name| attribute.path().is_ident(name))
        {
            return Err(syn::Error::new_spanned(
                attribute,
                format!(
                    "`#[{name}]` on a field was replaced by `#[slot]`; the field type \
                     already says what the slot points at"
                ),
            ));
        }
    }
    Ok(())
}

fn parse_slot(field: &Field) -> Result<Option<SlotField>> {
    let mut attributes = field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("slot"));
    let Some(attribute) = attributes.next() else {
        return Ok(None);
    };
    if let Some(repeated) = attributes.next() {
        return Err(syn::Error::new_spanned(
            repeated,
            "a field takes one `#[slot]` attribute",
        ));
    }
    let Some(field_ident) = field.ident.clone() else {
        return Err(syn::Error::new_spanned(
            attribute,
            "a slot is addressed by its field name; use a struct with named fields",
        ));
    };

    let mut key: Option<LitStr> = None;
    let mut purpose: Option<LitStr> = None;
    if !matches!(attribute.meta, syn::Meta::Path(_)) {
        attribute.parse_args_with(|input: ParseStream<'_>| {
            while !input.is_empty() {
                let name: Ident = input.parse()?;
                input.parse::<Token![=]>()?;
                let value: LitStr = input.parse()?;
                let target = match name.to_string().as_str() {
                    "key" => &mut key,
                    "purpose" => &mut purpose,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            &name,
                            "unknown `#[slot]` argument; expected `key` or `purpose`",
                        ));
                    },
                };
                if target.is_some() {
                    return Err(syn::Error::new_spanned(
                        &name,
                        "repeated `#[slot]` argument",
                    ));
                }
                *target = Some(value);
                if input.is_empty() {
                    break;
                }
                input.parse::<Token![,]>()?;
            }
            Ok(())
        })?;
    }

    let (key, span) = match key {
        Some(literal) => (literal.value(), literal.span()),
        None => (
            field_ident.to_string().trim_start_matches("r#").to_owned(),
            field_ident.span(),
        ),
    };
    if !is_valid_slot_key(&key) {
        return Err(syn::Error::new(
            span,
            format!(
                "slot key `{key}` is invalid: use 1 to 64 lowercase ASCII letters, digits \
                 and `_`, starting with a letter and not ending with `_`"
            ),
        ));
    }

    Ok(Some(SlotField {
        field_ident,
        ty: field.ty.clone(),
        key,
        purpose,
    }))
}

/// The rule of `nebula_component::is_valid_slot_key`, checked at expansion.
#[must_use]
pub fn is_valid_slot_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1] != b'_'
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// The body of `Component::dependencies` for `slots`.
///
/// `krate` is the path that re-exports `Dependencies` and `Slot`, such as
/// `::nebula_action`.
#[must_use]
pub fn emit_dependencies(krate: &TokenStream, slots: &[SlotField]) -> TokenStream {
    let calls = slots.iter().map(|slot| {
        let ty = &slot.ty;
        let key = LitStr::new(&slot.key, Span::call_site());
        let purpose = slot
            .purpose
            .as_ref()
            .map(|purpose| quote! { .with_purpose(#purpose) });
        quote! { .slot(#krate::Slot::of::<#ty>(#key) #purpose) }
    });
    quote! { #krate::Dependencies::none() #(#calls)* }
}

#[cfg(test)]
mod tests {
    use syn::DeriveInput;

    use super::*;

    fn parse(source: &str) -> Result<Vec<SlotField>> {
        let input: DeriveInput = syn::parse_str(source)?;
        let syn::Data::Struct(data) = input.data else {
            panic!("test requires a struct");
        };
        parse_slots(&data.fields)
    }

    #[test]
    fn the_key_defaults_to_the_field_name_and_the_type_is_kept_whole() {
        let slots = parse(
            "struct A { plain: u32, #[slot] db: ResourceHandle<Pg>, \
             #[slot(key = \"audit\", purpose = \"Audit log\")] r#log: Option<my::Alias> }",
        )
        .expect("valid slots");
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].key, "db");
        assert_eq!(slots[1].key, "audit");
        assert_eq!(
            slots[1].purpose.as_ref().map(LitStr::value).as_deref(),
            Some("Audit log")
        );
        let ty = &slots[1].ty;
        assert_eq!(quote!(#ty).to_string(), "Option < my :: Alias >");
    }

    #[test]
    fn a_raw_identifier_field_is_keyed_without_its_prefix() {
        let slots = parse("struct A { #[slot] r#type: ResourceHandle<Pg> }").expect("valid");
        assert_eq!(slots[0].key, "type");
    }

    #[test]
    fn malformed_declarations_are_refused() {
        for (source, message) in [
            (
                "struct A { #[slot] a: X, #[slot(key = \"a\")] b: X }",
                "declared twice",
            ),
            ("struct A { #[slot(key = \"Db\")] a: X }", "is invalid"),
            ("struct A { #[slot(key = \"db.main\")] a: X }", "is invalid"),
            (
                "struct A { #[slot(kind = \"x\")] a: X }",
                "unknown `#[slot]` argument",
            ),
            (
                "struct A { #[slot(key = \"a\", key = \"b\")] a: X }",
                "repeated `#[slot]` argument",
            ),
            (
                "struct A { #[slot] #[slot] a: X }",
                "one `#[slot]` attribute",
            ),
            ("struct A(#[slot] X);", "named fields"),
            ("struct A { #[resource] a: X }", "replaced by `#[slot]`"),
            (
                "struct A { #[credential(key = \"a\")] a: X }",
                "replaced by `#[slot]`",
            ),
        ] {
            let error = parse(source).expect_err(source);
            assert!(error.to_string().contains(message), "{source}: {error}");
        }
    }

    #[test]
    fn the_dependencies_body_names_each_field_type() {
        let slots = parse("struct A { #[slot(purpose = \"Main\")] db: ResourceHandle<Pg> }")
            .expect("valid");
        let body = emit_dependencies(&quote!(::nebula_action), &slots).to_string();
        assert_eq!(
            body,
            ":: nebula_action :: Dependencies :: none () . slot (:: nebula_action :: Slot :: of :: < ResourceHandle < Pg > > (\"db\") . with_purpose (\"Main\"))"
        );
    }
}
