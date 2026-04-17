//! Compile-time macros for nebula-schema.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Span;
use quote::quote;
use syn::{LitStr, parse_macro_input};

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

    // Resolve the crate path robustly: supports renamed dependencies.
    //
    // We do NOT use `crate::` for FoundCrate::Itself because proc-macro
    // doctests are compiled as external crates — `crate` would resolve to
    // the synthetic doctest crate, not to nebula_schema.  Using the
    // absolute path `::nebula_schema` is safe for all call sites.
    let crate_path = match crate_name("nebula-schema") {
        Ok(FoundCrate::Itself) | Err(_) => quote!(::nebula_schema),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote!(::#ident)
        },
    };

    let out = quote! {
        #crate_path::FieldKey::new(#lit)
            .expect("field_key! validated at compile time")
    };
    out.into()
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
