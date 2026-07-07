//! Proc-macro crate for the `Plugin` derive macro.
//!
//! Generates `Plugin` trait impl with `manifest()`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

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
/// - `key = "..."` — Unique plugin key (required)
/// - `name = "..."` — Human-readable name (required)
/// - `description = "..."` — Short description (optional)
/// - `version = "MAJOR.MINOR.PATCH"` — Semver version (default: `"1.0.0"`)
/// - `group = [...]` — Group hierarchy for UI (optional)
///
/// # Example
///
/// ```text
/// #[derive(Plugin)]
/// #[plugin(
///     key = "http",
///     name = "HTTP",
///     description = "HTTP request actions",
///     version = "2.0.0",
///     group = ["network", "api"]
/// )]
/// pub struct HttpPlugin;
/// ```
///
/// This crate is `proc-macro = true`, so it cannot depend on `nebula-plugin`
/// and the derive above cannot compile as a doctest here. For a runnable
/// example see `nebula_plugin::Plugin` in the parent `nebula-plugin` crate.
#[proc_macro_derive(Plugin, attributes(plugin))]
pub fn derive_plugin(input: TokenStream) -> TokenStream {
    plugin::derive(input)
}
