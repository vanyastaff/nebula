//! Proc-macro crate for the `Validator` derive macro.
//!
//! Implements `nebula_validator::foundation::Validate` for the struct and
//! generates an inherent `validate_fields()` helper.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod validator;

/// Derive macro for generating field-based validators.
///
/// See the `nebula-macros` crate documentation for the full list of
/// supported container and field attributes.
#[proc_macro_derive(Validator, attributes(validator, validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    validator::derive(input)
}
