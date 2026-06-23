//! Compile-time macros for nebula-schema.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{DeriveInput, LitStr, parse_macro_input};

mod attrs;
mod derive_enum;
mod derive_schema;
mod type_infer;

/// Resolve the path to the `nebula-schema` crate so the generated code
/// works whether the caller renamed the dependency or derives are
/// expanded from inside `nebula-schema` itself.
///
/// Resolution rules (driven by [`proc_macro_crate::crate_name`]):
/// - [`FoundCrate::Itself`] → `::nebula_schema` (works in lib code via `extern crate self as
///   nebula_schema;` in `lib.rs`, in doctests via the doctest binary's external crate alias, and in
///   integration tests / examples / benches via the package's own crate alias).
/// - [`FoundCrate::Name(name)`] → `::name` (external crate that renamed the dependency).
/// - `Err(_)` → fall back to `::nebula_schema` for unknown contexts.
pub(crate) fn crate_path() -> TokenStream2 {
    match crate_name("nebula-schema") {
        Ok(FoundCrate::Itself) | Err(_) => quote!(::nebula_schema),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote!(::#ident)
        },
    }
}

/// Build a `FieldKey` (from `nebula-schema`) from a string literal, using the same rules as
/// `FieldKey::new` at **compile time** (non-empty, max 64 chars, ASCII identifier: leading letter
/// or `_`, then letters, digits, or `_`).
///
/// ```ignore
/// let k = field_key!("alpha");   // OK
/// let k = field_key!("1bad");    // compile error
/// ```
#[proc_macro]
pub fn field_key(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    let value = lit.value();

    if let Err(msg) = validate_field_key(&value) {
        return syn::Error::new(lit.span(), format!("invalid FieldKey literal: {msg}"))
            .to_compile_error()
            .into();
    }

    let crate_path = crate_path();

    let out = quote! {
        #crate_path::FieldKey::new(#lit)
            .expect("field_key! validated at compile time")
    };
    out.into()
}

/// Derive `HasSchema` (from `nebula-schema`) for a struct.
///
/// Supported attributes:
/// - `#[field(...)]` — label/description/placeholder/default/hint/secret/
///   multiline/no_expression/expression_required/enum_select/skip/group.
/// - `#[validate(...)]` — required/length(min,max)/range(min..=max)/ pattern/url/email.
/// - `#[schema(...)]` — struct-level options: `custom = "..."` → the validator's
///   `Rule::custom` on the built schema (deferred wire hook); `reserved("a", "b")`
///   → keys that may not be used by any field (reusing a removed field's key would
///   misread older documents), rejected at expansion if a field collides.
/// - `#[serde(...)]` — read for key alignment so the schema key equals the wire
///   key: `rename` / `rename_all` rename the field, `skip` / `skip_deserializing`
///   drop it. `#[serde(flatten)]` is rejected (splicing is a follow-up).
#[proc_macro_derive(Schema, attributes(field, validate, schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_schema::expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Derive `HasSelectOptions` (from `nebula-schema`) for a unit-only enum.
/// Variant names become catalog values following serde (`rename` / `rename_all`,
/// else `snake_case`); `#[serde(skip)]` drops a variant. Use
/// `#[field(label = "...")]` to override the display label.
#[proc_macro_derive(EnumSelect, attributes(field))]
pub fn derive_enum_select(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_enum::expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Validate a candidate schema field key against the `FieldKey` rules
/// (non-empty, ≤64 chars, leading ASCII letter or `_`, then ASCII alphanumerics
/// or `_`). Shared by the `field_key!` macro and the `Schema` / `EnumSelect`
/// derives so the rules live in exactly one place (mirror of
/// `nebula_schema::FieldKey::new`).
pub(crate) fn validate_field_key(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("key cannot be empty");
    }
    if value.chars().count() > 64 {
        return Err("key max 64 chars");
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err("key must start with letter or underscore");
    }
    for ch in chars {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err("key must be ASCII alphanumeric or underscore");
        }
    }
    Ok(())
}
