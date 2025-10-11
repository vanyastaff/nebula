//! Shared utilities for proc-macros

use proc_macro2::Span;
use syn::{Ident, LitStr};

/// Creates an identifier with the given name at the call site span.
#[allow(dead_code)]
pub(crate) fn ident(name: &str) -> Ident {
    Ident::new(name, Span::call_site())
}

/// Creates a string literal with the given value at the call site span.
#[allow(dead_code)]
pub(crate) fn lit_str(value: &str) -> LitStr {
    LitStr::new(value, Span::call_site())
}

/// Converts a field name to a readable error field name.
///
/// Transforms snake_case field names to more human-readable format for error messages.
/// For example, "user_name" becomes "user name" in error messages.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(field_name_to_error_field("user_name"), "user name");
/// assert_eq!(field_name_to_error_field("email"), "email");
/// assert_eq!(field_name_to_error_field("first_name"), "first name");
/// ```
pub(crate) fn field_name_to_error_field(field: &str) -> String {
    // Convert snake_case to space-separated words for better readability
    field.replace('_', " ")
}

/// Pluralizes a word (simple English pluralization).
#[allow(dead_code)]
pub(crate) fn pluralize(word: &str) -> String {
    if word.ends_with('s') {
        format!("{word}es")
    } else if let Some(stripped) = word.strip_suffix('y') {
        format!("{stripped}ies")
    } else {
        format!("{word}s")
    }
}
