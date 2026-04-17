//! Compile-time macros for nebula-schema.

use proc_macro::TokenStream;
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

    let out = quote! {
        ::nebula_schema::FieldKey::new(#lit)
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
