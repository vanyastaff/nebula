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
/// Both [`FoundCrate::Itself`] and `Err(_)` map to the absolute path
/// `::nebula_schema`, **not** `crate` — `cargo test --doc` compiles
/// every doctest as a separate binary that links against nebula-schema
/// as an external dependency, so from that binary's perspective `crate`
/// resolves to the synthetic doctest crate, not to `nebula_schema`.
/// Using the absolute path is safe for all call sites (doctests,
/// integration tests, external crates) and was the original design —
/// restoring it here after an earlier review suggestion that missed
/// the doctest linkage.
pub(crate) fn crate_path() -> TokenStream2 {
    match crate_name("nebula-schema") {
        Ok(FoundCrate::Itself) | Err(_) => quote!(::nebula_schema),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote!(::#ident)
        },
    }
}

/// Build a `FieldKey` from a string literal, validated at compile time.
///
/// ```ignore
/// let k = field_key!("alpha");   // OK
/// let k = field_key!("1bad");    // compile error
/// ```
#[proc_macro]
pub fn field_key(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    let value = lit.value();

    if let Err(msg) = validate(&value) {
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
/// - `#[param(...)]` — label/description/placeholder/default/hint/secret/
///   multiline/no_expression/expression_required/enum_select/skip/group.
/// - `#[validate(...)]` — required/length(min,max)/range(min..=max)/ pattern/url/email.
///
/// Struct-level `#[schema(...)]` is reserved for a future pass (no options
/// functional today).
#[proc_macro_derive(Schema, attributes(param, validate))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_schema::expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Derive `HasSelectOptions` (from `nebula-schema`) for a unit-only enum.
/// Variant names snake_case into stored values; use
/// `#[param(label = "...")]` to override the display label.
#[proc_macro_derive(EnumSelect, attributes(param))]
pub fn derive_enum_select(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_enum::expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn validate(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("key cannot be empty");
    }
    if value.len() > 64 {
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
