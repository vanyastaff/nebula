//! Proc-macro crate for the `Parameters` derive macro.
//!
//! Generates `parameters()` and `param_count()` from struct fields.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod param_attrs;
mod parameter;

/// Derive macro for generating parameter definitions.
///
/// Generates `ParameterCollection` from struct fields and their attributes.
///
/// # Field Attributes
///
/// - `#[param(description = "...")]` - Field description (optional)
/// - `#[param(required)]` - Marks the field as required (default: optional)
/// - `#[param(secret)]` - Marks the field as sensitive data
/// - `#[param(default = ...)]` - Default value for the field
/// - `#[param(validation = "...")]` - Validation rule (email, url, regex, range)
/// - `#[param(options = [...])]` - Select options
///
/// # Example
///
/// ```ignore
/// #[derive(Parameters)]
/// pub struct DatabaseConfig {
///     #[param(description = "Database host", required, default = "localhost")]
///     host: String,
///
///     #[param(description = "Port number", validation = "range(1, 65535)", default = 5432)]
///     port: u16,
///
///     #[param(description = "Password", secret)]
///     password: String,
/// }
/// ```
#[proc_macro_derive(Parameters, attributes(param))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    parameter::derive(input)
}
