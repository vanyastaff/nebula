//! Proc-macro crate for the `Plugin` derive macro.
//!
//! Generates `Plugin` trait impl with `metadata()`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod plugin;
mod plugin_attrs;

/// Derive macro for the `Plugin` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[plugin(...)]` on the struct)
///
/// - `key = "..."` - Unique plugin key (required)
/// - `name = "..."` - Human-readable name (required)
/// - `description = "..."` - Short description (optional)
/// - `version = N` - Version number (default: 1)
/// - `group = [...]` - Group hierarchy for UI (optional)
///
/// # Example
///
/// ```ignore
/// #[derive(Plugin)]
/// #[plugin(
///     key = "http",
///     name = "HTTP",
///     description = "HTTP request actions",
///     version = 2,
///     group = ["network", "api"]
/// )]
/// pub struct HttpPlugin;
/// ```
#[proc_macro_derive(Plugin, attributes(plugin))]
pub fn derive_plugin(input: TokenStream) -> TokenStream {
    plugin::derive(input)
}
