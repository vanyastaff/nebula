//! # nebula-error-macros
//!
//! Proc-macros for the [`nebula-error`] crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;
use proc_macro::TokenStream;

/// Derive the `Classify` trait for an error enum.
///
/// See `nebula_error::Classify` for details.
#[proc_macro_derive(Classify, attributes(classify))]
pub fn derive_classify(input: TokenStream) -> TokenStream {
    let _ = input;
    TokenStream::new()
}
