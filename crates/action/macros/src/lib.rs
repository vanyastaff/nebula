//! Proc-macro crate for the `Action` derive macro and `#[action]` attribute macro.
//!
//! - `#[derive(Action)]` generates `DeclaresDependencies` and `Action` trait impls with
//!   `metadata()`.
//! - `#[action]` rewrites `CredentialRef<dyn X>` field types to `CredentialRef<dyn XPhantom>` per
//!   ADR-0035 4.3 + Tech Spec 2.7.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod action;
mod action_attr;
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

/// Attribute macro for action structs with capability-bound credential fields.
///
/// Rewrites every `CredentialRef<dyn X>` field on the annotated struct to
/// `CredentialRef<dyn XPhantom>` in the emitted item. Concrete-typed
/// fields (Pattern 1: `CredentialRef<ConcreteCredential>`) and any
/// non-`CredentialRef` field are pass-through.
///
/// Apply *before* `#[derive(Action)]` when both are needed - the
/// attribute rewrites first, then the derive sees the rewritten field
/// types and generates impls against the phantom-suffixed `dyn`.
///
/// # Example
///
/// ```ignore
/// use nebula_action_macros::{action, Action};
/// use nebula_credential::CredentialRef;
///
/// #[action]
/// #[derive(Action)]
/// #[action(key = "bitbucket.repo.fetch", name = "Fetch Repo")]
/// pub struct BitbucketRepoFetch {
///     pub bb: CredentialRef<dyn BitbucketBearer>,
///     // - emitted as `CredentialRef<dyn BitbucketBearerPhantom>`
/// }
/// ```
///
/// # Diagnostics
///
/// The rewrite is silent - Tech Spec 2.7 line 487 codifies "rewrites
/// silently". Pattern 1 fields and non-`CredentialRef` fields surface
/// the standard rustc diagnostics for any errors in their declaration;
/// no extra error emission from this macro.
#[proc_macro_attribute]
pub fn action(args: TokenStream, input: TokenStream) -> TokenStream {
    action_attr::expand(args, input)
}
