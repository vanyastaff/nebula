use proc_macro::TokenStream;

/// Convert `syn::Error` into a TokenStream that emits a proper compiler error.
///
/// Keep this in one place to have consistent diagnostics across all macros.
pub fn to_compile_error(err: syn::Error) -> TokenStream {
    // syn::Error already contains spans and can render to compile_error!
    err.to_compile_error().into()
}

/// Convenience: create a new `syn::Error` with the given span + message.
pub fn error_spanned<T: quote::ToTokens>(tokens: &T, msg: impl Into<String>) -> syn::Error {
    syn::Error::new_spanned(tokens, msg.into())
}
