//! Proc-macro crate for `Parameters` and `EnumSelect` derive macros.
//!
//! - `#[derive(Parameters)]` generates `HasParameters` from struct fields
//! - `#[derive(EnumSelect)]` generates `HasSelectOptions` from enum variants

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod enum_select;
mod param_attrs;
mod parameter;

/// Derive macro for generating parameter definitions from struct fields.
///
/// Generates `HasParameters` trait impl and inherent `parameters()` method.
/// Infers `ParameterType` from Rust types: `String` → string, `bool` → boolean,
/// `u32`/`i64` → integer, `f64` → number, `Option<T>` → optional,
/// `Vec<T>` → list, enum with `EnumSelect` → select.
///
/// # Field Attributes
///
/// `#[param(...)]`:
/// - `label = "..."` — display label
/// - `description = "..."` — help text
/// - `placeholder = "..."` — placeholder text
/// - `hint = "url"` — input hint (url, email, date, etc.)
/// - `default = value` — default value
/// - `required` — mark as required
/// - `secret` — mask in UI
/// - `multiline` — textarea mode (String only)
/// - `no_expression` — disable expression toggle
/// - `skip` — exclude from schema
///
/// # Example
///
/// ```ignore
/// #[derive(Parameters)]
/// struct HttpRequestInput {
///     #[param(label = "URL", hint = "url")]
///     url: String,
///
///     #[param(default = "GET")]
///     method: HttpMethod,
///
///     #[param(label = "Timeout (s)")]
///     timeout: Option<u32>,
///
///     #[param(skip)]
///     _internal: (),
/// }
/// ```
#[proc_macro_derive(Parameters, attributes(param, validate))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    parameter::derive(input)
}

/// Derive macro for generating select options from enum variants.
///
/// Generates `HasSelectOptions` and `InferParameterType` trait impls.
/// Only supports unit variants (no fields).
///
/// # Variant Attributes
///
/// `#[param(...)]`:
/// - `label = "..."` — display label (defaults to variant name)
/// - `description = "..."` — option description
///
/// # Example
///
/// ```ignore
/// #[derive(EnumSelect)]
/// enum HttpMethod {
///     #[param(label = "GET")]
///     Get,
///     #[param(label = "POST")]
///     Post,
/// }
/// ```
#[proc_macro_derive(EnumSelect, attributes(param))]
pub fn derive_enum_select(input: TokenStream) -> TokenStream {
    enum_select::derive(input)
}
