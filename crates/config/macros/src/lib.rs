//! Proc-macro crate for the `Config` derive macro.
//!
//! Generates `from_env()`, `load()`, and `validate_fields()` methods,
//! plus a `Validate` impl.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod config;

/// Derive macro for env-backed configuration types with field validation.
///
/// See the `nebula-macros` crate documentation for the full list of
/// supported container and field attributes.
#[proc_macro_derive(Config, attributes(config, validator, validate))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    config::derive(input)
}
