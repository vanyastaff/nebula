//! Proc-macro crate for the `Validator` derive macro.
//!
//! Implements `nebula_validator::foundation::Validate` for the struct and
//! generates an inherent `validate_fields()` helper.
//!
//! ## Architecture
//!
//! Three-phase pipeline: `parse` (attributes → IR) → `emit` (IR → TokenStream).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod emit;
mod model;
mod parse;

/// Derive macro for generating field-based validators.
///
/// See the `nebula-macros` crate documentation for the full list of
/// supported container and field attributes.
#[proc_macro_derive(Validator, attributes(validator, validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => nebula_macro_support::diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ir = parse::parse(&input)?;
    Ok(emit::emit(&ir))
}
