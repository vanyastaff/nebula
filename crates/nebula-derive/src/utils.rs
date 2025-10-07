//! Shared utilities for proc-macros

use proc_macro2::Span;
use syn::{Ident, LitStr};

/// Creates an identifier with the given name at the call site span.
#[allow(dead_code)]
pub fn ident(name: &str) -> Ident {
    Ident::new(name, Span::call_site())
}

/// Creates a string literal with the given value at the call site span.
#[allow(dead_code)]
pub fn lit_str(value: &str) -> LitStr {
    LitStr::new(value, Span::call_site())
}

/// Converts a field name to a readable error field name.
///
/// # Examples
///
/// ```
/// assert_eq!(field_name_to_error_field("user_name"), "user_name");
/// assert_eq!(field_name_to_error_field("email"), "email");
/// ```
#[allow(dead_code)]
pub fn field_name_to_error_field(field: &str) -> String {
    field.to_string()
}

/// Pluralizes a word (simple English pluralization).
#[allow(dead_code)]
pub fn pluralize(word: &str) -> String {
    if word.ends_with('s') {
        format!("{}es", word)
    } else if word.ends_with('y') {
        format!("{}ies", &word[..word.len() - 1])
    } else {
        format!("{}s", word)
    }
}
