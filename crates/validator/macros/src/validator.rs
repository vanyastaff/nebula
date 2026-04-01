//! Validator derive macro implementation — 3-phase pipeline.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

use nebula_macro_support::diag;

use crate::{emit, parse};

/// Entry point for `#[derive(Validator)]`.
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    // Phase 1: Parse attributes into IR
    let ir = parse::parse(&input)?;

    // Phase 2: (semantic checks are done during parsing)

    // Phase 3: Generate code from IR
    Ok(emit::emit(&ir))
}
