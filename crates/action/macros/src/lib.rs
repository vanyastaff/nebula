//! Proc-macro crate for the `Action` derive macro.
//!
//! Generates `DeclaresDependencies` and `Action` trait impls with `metadata()`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod action;
mod action_attrs;

/// Derive macro for the `Action` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[action(...)]` on the struct)
///
/// - `key = "..."` - Unique identifier for this action (required)
/// - `name = "..."` - Human-readable name (required)
/// - `description = "..."` - Short description (required)
/// - `version = "..."` - Interface version, e.g., "1.0" (default: "1.0")
/// - `credential = Type` - Single credential type for `DeclaresDependencies` (optional)
/// - `credentials = [Type1, Type2]` - Multiple credential types (optional)
/// - `resource = Type` - Single resource type for `DeclaresDependencies` (optional)
/// - `resources = [Type1, Type2]` - Multiple resource types (optional)
/// - `parameters = Type` - Type with `parameters()` for `ActionMetadata` (optional)
///
/// Note: `credential = "key"` (string) is ignored; use `credential = CredentialType` for type-based
/// refs.
///
/// Action structs must be unit structs with no fields (e.g. `struct MyAction;`).
///
/// # Example
///
/// ```ignore
/// #[derive(Action)]
/// #[action(
///     key = "slack.send",
///     name = "Send Slack Message",
///     description = "Sends a message to a Slack channel",
///     version = "2.1",
///     credential = SlackOAuthCredential,
///     resources = [HttpClient]
/// )]
/// pub struct SlackSendAction;
/// ```
#[proc_macro_derive(Action, attributes(action, nebula))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    action::derive(input)
}
