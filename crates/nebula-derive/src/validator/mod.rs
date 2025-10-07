//! Validator derive macro implementation

mod parse;
mod generate;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

/// Main entry point for the Validator derive macro.
pub fn derive_validator_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match generate::generate_validator(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
